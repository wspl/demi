import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, realpath, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { CommandContext } from '@demicodes/command-protocol'
import {
  connectTestRunner,
  nativeCommandFixture,
  TEST_COMMAND_CONTEXT,
  TEST_RUNNER_DEVICE,
} from '@demicodes/host-remote/testing'
import { memoryHostStore } from '@demicodes/shell/testing'
import { collectBytes, decodeUtf8, waitFor } from '@demicodes/utils'
import { PipeBroker, RemoteHost, RemoteServiceError, devicePipes } from '@demicodes/host-remote'
import type { RunnerToBackendMessage } from '@demicodes/runner-protocol'

// The runner's service streams (`runner.md` § Service streams): `service_open`
// invokes a declared operation of a resident native service and carries its
// input and standard output as two pipes. The service is the runner's test
// fixture, which a fresh runner first installs through the stream's own
// artifact request.

const cleanups: Array<() => Promise<void>> = []
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup()
})

/** The live view's caller: the conversation's user. */
const context: CommandContext = { ...TEST_COMMAND_CONTEXT, caller: { kind: 'user' } }

async function connected() {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'demi-service-streams-')))
  cleanups.push(() => rm(dir, { recursive: true, force: true }))
  const native = await nativeCommandFixture()
  const pipes = new PipeBroker()
  const received: RunnerToBackendMessage[] = []
  let remote!: RemoteHost
  const connection = await connectTestRunner({
    home: dir,
    pipes,
    onHello(send, hello) {
      remote = new RemoteHost({
        defaultCwd: dir,
        identity: hello.runner.identity,
        store: memoryHostStore(),
        pipes: devicePipes(pipes, TEST_RUNNER_DEVICE),
      })
      remote.attach(send)
    },
    onMessage: (message) => {
      received.push(message)
      remote.handleMessage(message)
    },
    onClose: () => remote.detach('runner disconnected'),
  })
  cleanups.push(connection.close)
  const open = async (operation: string) => {
    const input = pipes.open(undefined, { deviceId: TEST_RUNNER_DEVICE })
    const output = pipes.open({ deviceId: TEST_RUNNER_DEVICE })
    const stream = await remote.services.open({
      context,
      package: native.descriptor,
      operation,
      cwd: dir,
      input: input.ref(),
      output: output.ref(),
      resolveArtifact: native.resolveArtifact,
    })
    return { input, output, stream }
  }
  return { dir, open, received, pipes }
}

test(
  'a service stream invokes the operation with its context and cwd, carries bytes both ways, and ends with the invocation',
  async () => {
    const { dir, open, received } = await connected()

    // The context and the directory reach the invocation; its environment is
    // empty. Its completion ends the output pipe.
    const where = await open('where')
    expect(JSON.parse(decodeUtf8(await collectBytes(where.output.stream())))).toEqual({
      label: null,
      context,
      cwd: dir,
      value: null,
    })
    where.stream.close()
    // The first stream installed the fixture through its own artifact request.
    expect(received.some(message => message.type === 'artifact_resolve'
      && 'streamId' in message.owner)).toBe(true)

    // The page's bytes arrive as the operation asks for them, in chunks the
    // protocol allows, and come back byte-equal; input EOF completes it.
    const echo = await open('echo')
    const payload = new Uint8Array(1024 * 1024).map((_, index) => (index * 31) % 251)
    const returned = collectBytes(echo.output.stream())
    const writer = echo.input.writer()
    await writer.write(payload)
    writer.end()
    expect(await returned).toEqual(payload)
    await echo.output.done
    echo.stream.close()

    // A package without the operation refuses the stream before any byte.
    const refused = await open('missing').then(() => null, (error: unknown) => error)
    expect(refused).toBeInstanceOf(RemoteServiceError)
    expect((refused as RemoteServiceError).code).toBe('unknown_operation')
  },
  60_000,
)

test(
  'the page going away cancels the invocation, and the runner reports both pipes',
  async () => {
    const { open, received, pipes } = await connected()
    const held = await open('echo')
    const output = collectBytes(held.output.stream())
    const writer = held.input.writer()
    await writer.write(new TextEncoder().encode('before the page left'))
    pipes.fail(held.input.id, 'page closed')
    expect(await output.then(() => 'ended', () => 'failed')).toBe('failed')
    await waitFor(() => [held.input.id, held.output.id].every(id => received.some(
      message => message.type === 'pipe_done' && message.pipeId === id && !message.ok,
    )))
    held.stream.close()
  },
  60_000,
)
