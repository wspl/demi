import type { RemoteHost } from '@demicodes/host-remote'
import { expect, test } from 'bun:test'
import {
  ClaudeCli,
  ClaudeCliError,
  ClaudeReleases,
  type ClaudeCliTarget
} from '../llm/claude-cli'

const BASE = 'https://downloads.claude.ai/claude-code-releases'
const sha = (seed: string) => seed.repeat(64).slice(0, 64)

/** The vendor's distribution: a `latest` pointer and one manifest per version. */
function distribution(state: { latest: string; versions: string[] }) {
  const requests: string[] = []
  const answer = (async (input: string | URL | Request) => {
    const url = String(input)
    requests.push(url.slice(BASE.length))
    if (url === `${BASE}/latest`)
      return new Response(`${state.latest}\n`)
    const version = state.versions.find(v => url === `${BASE}/${v}/manifest.json`)
    if (!version)
      return new Response('missing', { status: 404 })
    return Response.json({
      version,
      commit: 'ignored',
      platforms: {
        'darwin-arm64': { binary: 'claude', checksum: sha('a'), size: 10 },
        'win32-x64': { binary: 'claude.exe', checksum: sha('b'), size: 11 },
      },
    })
  }) as typeof fetch
  return { requests, fetch: answer }
}

/** A machine whose `demi.claude` package answers from memory. */
function machine(installed: string[], fail?: { code: string; message: string }) {
  const calls: Array<{ operation: string; input: unknown }> = []
  const host = {
    services: {
      call: async (params: { operation: string; input: Uint8Array }) => {
        const text = new TextDecoder().decode(params.input)
        calls.push({ operation: params.operation, input: text ? JSON.parse(text) : null })
        const reply = (value: unknown) => new TextEncoder().encode(`${JSON.stringify(value)}\n`)
        if (params.operation === 'claude.status')
          return reply({
            ok: true,
            platform: 'darwin-arm64',
            installed: installed.map(version => ({ version, path: `/home/.demi/claude/${version}/claude` }))
          })
        if (fail)
          return reply({ ok: false, ...fail })
        const { version } = JSON.parse(text) as { version: string }
        installed.unshift(version)
        return reply({ ok: true, version, path: `/home/.demi/claude/${version}/claude` })
      },
    },
  } as unknown as RemoteHost
  const target: ClaudeCliTarget = {
    host,
    deviceId: 'device',
    cwd: '/home',
    context: {
      conversation: 'c',
      caller: { kind: 'user' },
      locale: { timeZone: 'UTC', languages: ['en'] }
    },
  }
  return { calls, target }
}

const cliOver = (vendor: ReturnType<typeof distribution>, now = () => 0) => new ClaudeCli({
  releases: new ClaudeReleases({ fetch: vendor.fetch, now }),
  package: { id: 'demi.claude' } as never,
  resolveArtifact: async () => ({ path: '/unused' }),
})

test('the wanted version is the vendor\'s newest, read from its manifest and believed for a while', async () => {
  let now = 0
  const vendor = distribution({ latest: '2.1.5', versions: ['2.1.5', '2.1.6'] })
  const releases = new ClaudeReleases({ fetch: vendor.fetch, now: () => now })
  const release = await releases.latest()
  expect(release.version).toBe('2.1.5')
  expect(release.platforms['win32-x64']).toEqual({
    url: `${BASE}/2.1.5/win32-x64/claude.exe`,
    size: 11,
    sha256: sha('b')
  })
  await releases.latest()
  expect(vendor.requests).toEqual(['/latest', '/2.1.5/manifest.json'])
  // Check for updates reads the pointer at once; so does the end of the while.
  vendor.requests.length = 0
  await releases.latest(true)
  expect(vendor.requests).toEqual(['/latest'])
  now += 7 * 60 * 60_000
  await releases.latest()
  expect(vendor.requests).toEqual(['/latest', '/latest'])
  await expect(releases.release('../etc')).rejects.toThrow('not a version')
  await expect(releases.release('9.9.9')).rejects.toBeInstanceOf(ClaudeCliError)
})

test('a machine with no CLI waits for the install; one with an older CLI answers with it and updates beside it', async () => {
  const vendor = distribution({ latest: '2.1.6', versions: ['2.1.5', '2.1.6'] })
  const cli = cliOver(vendor)

  const empty = machine([])
  expect(await cli.executable(empty.target)).toBe('/home/.demi/claude/2.1.6/claude')
  expect(empty.calls.map(call => call.operation)).toEqual(['claude.status', 'claude.ensure'])
  expect(empty.calls[1]!.input).toMatchObject({ version: '2.1.6' })

  const older = machine(['2.1.5'])
  expect(await cli.executable(older.target)).toBe('/home/.demi/claude/2.1.5/claude')
  await Bun.sleep(5)
  expect(older.calls.map(call => call.operation))
    .toEqual(['claude.status', 'claude.ensure'])
  // The next process is the newer one.
  expect(await cli.executable(older.target)).toBe('/home/.demi/claude/2.1.6/claude')

  const current = machine(['2.1.6'])
  await cli.executable(current.target)
  expect(current.calls.map(call => call.operation)).toEqual(['claude.status'])
})

test('a held entry wants its version only, and a failed install says why with the version', async () => {
  const vendor = distribution({ latest: '2.1.6', versions: ['2.1.5', '2.1.6'] })
  const held = machine(['2.1.6'])
  expect(await cliOver(vendor).executable(held.target, { held: '2.1.5' }))
    .toBe('/home/.demi/claude/2.1.5/claude')
  expect(vendor.requests).toEqual(['/2.1.5/manifest.json'])

  const broken = machine([], { code: 'unsupported_platform', message: 'no build for plan9-x64' })
  const failure = await cliOver(vendor).executable(broken.target).catch(error => error)
  expect(failure).toBeInstanceOf(ClaudeCliError)
  expect(failure.code).toBe('unsupported_platform')
  expect(failure.message).toBe('Claude Code 2.1.6 could not be installed: no build for plan9-x64')
})
