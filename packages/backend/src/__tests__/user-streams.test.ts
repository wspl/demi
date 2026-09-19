import { afterAll, beforeAll, expect, test } from 'bun:test'
import { nativeCommandFixture, nativePackageFixture } from '@demicodes/host-remote/testing'
import { waitFor } from '@demicodes/utils'
import { DEFAULT_LOCALE } from '../runner/command-context'
import type { Driver } from './scenarios/driver'
import { World } from './scenarios/world'

// User streams (`web-api.md` § User streams) end to end: the page's socket,
// the backend's admission and relay, the runner's service stream and a
// resident native service. The streams bind the runner's test fixture: `echo`
// returns what the page sends, `where` reports its context and directory.

let world: World

beforeAll(async () => {
  const builtin = await nativePackageFixture()
  const fixture = await nativeCommandFixture()
  const fixtureDigests = new Set(Object.values(fixture.descriptor.targets).map(artifact => artifact.sha256))
  world = await World.create({
    runners: ['alpha'],
    nativeCommands: {
      packages: [...builtin.packages, fixture.descriptor],
      resolveArtifact: (artifact, signal) => fixtureDigests.has(artifact.sha256)
        ? fixture.resolveArtifact(artifact, signal)
        : builtin.resolveArtifact(artifact, signal),
    },
    userStreams: {
      echo: { package: fixture.descriptor.id, operation: 'echo' },
      where: { package: fixture.descriptor.id, operation: 'where' },
    },
  })
}, 120_000)

afterAll(async () => {
  await world.close()
})

function path(driver: Driver, name: string): string {
  return `/api/conversations/${driver.id}/streams/${name}`
}

function socket(driver: Driver, name: string): WebSocket {
  const opened = new WebSocket(`${world.url.replace(/^http/, 'ws')}${path(driver, name)}`, {
    headers: { cookie: world.backend.session.cookie, origin: world.url },
  })
  opened.binaryType = 'arraybuffer'
  return opened
}

/** What the page receives as it arrives, and how the backend closed the socket. */
function received(opened: WebSocket) {
  const chunks: Uint8Array[] = []
  opened.addEventListener('message', event => chunks.push(new Uint8Array(event.data as ArrayBuffer)))
  const bytes = () => new Uint8Array(chunks.flatMap(chunk => [...chunk]))
  return {
    length: () => chunks.reduce((total, chunk) => total + chunk.length, 0),
    bytes,
    closed: new Promise<{ bytes: Uint8Array; code: number; reason: string }>(resolve =>
      opened.addEventListener('close', event => resolve({ bytes: bytes(), code: event.code, reason: event.reason }))),
  }
}

/** How the route answers before an upgrade. */
async function refusal(driver: Driver, name: string, origin: string | null = world.url) {
  const response = await world.backend.session.fetch(path(driver, name), {
    headers: {
      upgrade: 'websocket',
      connection: 'Upgrade',
      'sec-websocket-version': '13',
      'sec-websocket-key': btoa('0123456789abcdef'),
      ...(origin ? { origin } : {}),
    },
  })
  return { status: response.status, code: ((await response.json()) as { code: string }).code }
}

test('a page opens a user stream on the conversation\'s Host, with the user\'s context and the conversation\'s directory', async () => {
  const driver = await world.conversation('runner:alpha')
  const where = await received(socket(driver, 'where')).closed
  expect(where.code).toBe(1000)
  expect(where.reason).toBe('completed')
  const reported = JSON.parse(new TextDecoder().decode(where.bytes))
  expect(reported.context).toEqual({
    conversation: driver.id,
    caller: { kind: 'user' },
    locale: DEFAULT_LOCALE,
  })
  expect(reported.cwd).toBe(world.device('alpha').home)
  expect(reported.value).toBeNull()

  // Bytes go both ways as they are, in order, whatever the message
  // boundaries; the page closing its socket ends the stream.
  const echo = socket(driver, 'echo')
  const echoed = received(echo)
  const payload = new Uint8Array(3 * 1024 * 1024).map((_, index) => (index * 7) % 256)
  await new Promise(resolve => echo.addEventListener('open', resolve))
  for (let offset = 0; offset < payload.length; offset += 100_000)
    echo.send(payload.subarray(offset, offset + 100_000))
  await waitFor(() => echoed.length() === payload.length, () => `${echoed.length()} bytes back`, { timeoutMs: 20_000 })
  expect(echoed.bytes()).toEqual(payload)
  echo.close()
  expect((await echoed.closed).bytes).toEqual(payload)
}, 60_000)

test('the route refuses a foreign origin, an unknown stream, an archived conversation and an offline device', async () => {
  const driver = await world.conversation('runner:alpha')
  expect(await refusal(driver, 'echo', 'https://elsewhere.example')).toEqual({ status: 403, code: 'forbidden_origin' })
  expect(await refusal(driver, 'echo', null)).toEqual({ status: 403, code: 'forbidden_origin' })
  expect(await refusal(driver, 'browser')).toEqual({ status: 404, code: 'unknown_stream' })

  await world.killRunner('alpha')
  expect(await refusal(driver, 'echo')).toEqual({ status: 409, code: 'device_offline' })
  await world.returnRunner('alpha')

  await world.api(`/api/conversations/${driver.id}`, { archived: true }, 'PATCH')
  expect(await refusal(driver, 'echo')).toEqual({ status: 409, code: 'conversation_archived' })
}, 60_000)

test('an archive ends the conversation\'s open streams', async () => {
  const driver = await world.conversation('runner:alpha')
  const echo = socket(driver, 'echo')
  const ended = received(echo).closed
  await new Promise(resolve => echo.addEventListener('open', resolve))
  await world.api(`/api/conversations/${driver.id}`, { archived: true }, 'PATCH')
  expect(await ended).toMatchObject({ code: 4000, reason: 'conversation_changed' })
}, 60_000)
