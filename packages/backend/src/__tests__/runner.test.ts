import { existsSync } from 'node:fs'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { LocalHost } from '@demicodes/host-local'
import { encodeRunnerMessage } from '@demicodes/runner-protocol'
import { RunnerClient } from '@demicodes/runner'
import { delay, waitFor } from '@demicodes/utils'
import { createBackend, type Backend } from '../index'

// M4: the pairing flow end-to-end — unclaimed runner prints a code, the web
// API claims it, the device token round-trips a restart, revoke refuses the
// reconnect, and the browse endpoints ride the same Host RPC.

const NO_PROVIDERS: never[] = []

async function api(backend: Backend, path: string, init?: RequestInit): Promise<Response> {
  return fetch(`${backend.url}${path}`, init)
}

function startRunner(backend: Backend, dirs: { stateDir: string; runnerDir: string }, capture: RunnerCapture) {
  const runner = new RunnerClient({
    backendUrl: backend.url,
    stateDir: dirs.stateDir,
    name: 'test-device',
    host: new LocalHost(dirs.runnerDir),
    reconnect: { initialDelayMs: 30, maxDelayMs: 100 },
    onStatus: (status, detail) => {
      capture.statuses.push(status)
      if (detail) capture.details.push(detail)
    },
    onClaimPending: (code) => {
      capture.codes.push(code)
    },
  })
  runner.start()
  return runner
}

interface RunnerCapture {
  codes: string[]
  statuses: string[]
  details: string[]
}

function capture(): RunnerCapture {
  return { codes: [], statuses: [], details: [] }
}

test('pairing: claim, reconnect with the device token, revoke refuses the reconnect', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m4-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m4-runner-'))
  const backend = await createBackend({ dataDir, providers: NO_PROVIDERS, port: 0, runner: { pingIntervalMs: 0 } })

  const first = capture()
  const runner = startRunner(backend, { stateDir, runnerDir }, first)
  await waitFor(() => first.codes.length > 0, undefined, { timeoutMs: 5_000 })
  const code = first.codes[0]
  expect(code).toMatch(/^[0-9A-Z]{4}(-[0-9A-Z]{1,4}){6}$|^[0-9A-Z-]{26,}$/)

  // A wrong code is refused; the right one claims — entered messy, normalized server-side.
  const wrong = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: 'AAAA-BBBB' }),
    headers: { 'content-type': 'application/json' },
  })
  expect(wrong.status).toBe(404)
  const claimed = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: ` ${code.toLowerCase()} ` }),
    headers: { 'content-type': 'application/json' },
  })
  expect(claimed.status).toBe(201)
  const { device } = (await claimed.json()) as { device: { id: string; name: string; online: boolean } }
  expect(device.name).toBe('test-device')
  await waitFor(() => first.statuses.includes('online'))

  // Claiming is single-use: the spent code is gone.
  const reuse = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code }),
    headers: { 'content-type': 'application/json' },
  })
  expect(reuse.status).toBe(404)

  const listed = (await (await api(backend, '/api/devices')).json()) as {
    devices: Array<{ id: string; online: boolean }>
  }
  expect(listed.devices).toEqual([expect.objectContaining({ id: device.id, online: true })])

  // Directory browse and create ride the runner's Host RPC.
  await writeFile(join(runnerDir, 'hello.txt'), 'hi')
  const browse = (await (
    await api(backend, `/api/devices/${device.id}/fs?path=${encodeURIComponent(runnerDir)}`)
  ).json()) as { entries: Array<{ name: string; isDirectory: boolean }> }
  expect(browse.entries).toContainEqual({ name: 'hello.txt', isDirectory: false })
  const makeDir = await api(backend, `/api/devices/${device.id}/fs`, {
    method: 'POST',
    body: JSON.stringify({ path: join(runnerDir, 'made/by/web') }),
    headers: { 'content-type': 'application/json' },
  })
  expect(makeDir.status).toBe(201)
  expect(existsSync(join(runnerDir, 'made/by/web'))).toBe(true)

  // Restart the runner: the persisted device token authenticates directly.
  await runner.stop()
  let offline = false
  for (let tries = 0; tries < 100 && !offline; tries += 1) {
    const rows = (await (await api(backend, '/api/devices')).json()) as { devices: Array<{ online: boolean }> }
    offline = rows.devices[0]?.online === false
    if (!offline) await delay(20)
  }
  expect(offline).toBe(true)
  const second = capture()
  const restarted = startRunner(backend, { stateDir, runnerDir }, second)
  await waitFor(() => second.statuses.includes('online'), undefined, { timeoutMs: 5_000 })
  expect(second.codes).toHaveLength(0)

  // Revoke: the row disappears and the live connection is refused for good.
  const revoked = await api(backend, `/api/devices/${device.id}`, { method: 'DELETE' })
  expect(revoked.status).toBe(204)
  await waitFor(() => second.statuses.includes('rejected'))
  expect(second.details.some((detail) => detail.includes('revoked'))).toBe(true)
  const after = (await (await api(backend, '/api/devices')).json()) as { devices: unknown[] }
  expect(after.devices).toHaveLength(0)

  await restarted.stop()
  await backend.close()
}, 30_000)

test('claim codes expire and rotate on the waiting socket; the stale code is dead', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-expiry-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-m4-expiry-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-m4-expiry-runner-'))
  const backend = await createBackend({
    dataDir,
    providers: NO_PROVIDERS,
    port: 0,
    runner: { pingIntervalMs: 0, claimTtlMs: 150 },
  })
  const seen = capture()
  const runner = startRunner(backend, { stateDir, runnerDir }, seen)
  await waitFor(() => seen.codes.length >= 2, undefined, { timeoutMs: 5_000 })
  expect(seen.codes[1]).not.toBe(seen.codes[0])

  const stale = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: seen.codes[0] }),
    headers: { 'content-type': 'application/json' },
  })
  expect(stale.status).toBe(404)
  const fresh = await api(backend, '/api/devices/claim', {
    method: 'POST',
    body: JSON.stringify({ code: seen.codes[seen.codes.length - 1] }),
    headers: { 'content-type': 'application/json' },
  })
  expect(fresh.status).toBe(201)

  await runner.stop()
  await backend.close()
}, 15_000)

test('the claim endpoint is rate-limited per user', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-rate-'))
  const backend = await createBackend({
    dataDir,
    providers: NO_PROVIDERS,
    port: 0,
    runner: { pingIntervalMs: 0, claimAttemptsPerMinute: 2 },
  })
  const attempt = () =>
    api(backend, '/api/devices/claim', {
      method: 'POST',
      body: JSON.stringify({ code: 'NOPE-NOPE' }),
      headers: { 'content-type': 'application/json' },
    })
  expect((await attempt()).status).toBe(404)
  expect((await attempt()).status).toBe(404)
  expect((await attempt()).status).toBe(429)
  await backend.close()
})

test('a malformed runner frame closes the socket; a bad device token is rejected', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m4-reject-'))
  const backend = await createBackend({ dataDir, providers: NO_PROVIDERS, port: 0, runner: { pingIntervalMs: 0 } })
  const wsUrl = `${backend.url.replace('http', 'ws')}/api/runner`

  const garbage = new WebSocket(wsUrl)
  await new Promise<void>((resolve) => garbage.addEventListener('open', () => resolve(), { once: true }))
  const closed = new Promise<void>((resolve) => garbage.addEventListener('close', () => resolve(), { once: true }))
  garbage.send('{"type":"not-a-runner-frame"}')
  await closed

  const badToken = new WebSocket(wsUrl)
  await new Promise<void>((resolve) => badToken.addEventListener('open', () => resolve(), { once: true }))
  const reply = new Promise<string>((resolve) =>
    badToken.addEventListener('message', (event) => resolve(String(event.data)), { once: true }),
  )
  badToken.send(
    encodeRunnerMessage({
      type: 'hello',
      protocol: 1,
      deviceToken: 'not-a-real-token',
      runner: { name: 'x', platform: 'test', version: '0', identity: { uid: 0, gid: 0, hostname: 'x' } },
    }),
  )
  expect(JSON.parse(await reply)).toMatchObject({ type: 'hello_error', reason: 'unknown device' })
  badToken.close()
  await backend.close()
}, 15_000)
