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
import { collectBytes, decodeUtf8, delay, waitFor } from '@demicodes/utils'
import { PipeBroker, RemoteHost, RemoteServiceError, RemoteServiceExit, devicePipes } from '@demicodes/host-remote'
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
  const call = (operation: string, args?: Record<string, unknown>) => remote.services.call({
    context,
    package: native.descriptor,
    operation,
    ...(args ? { args, json: true } : {}),
    cwd: dir,
    input: new Uint8Array(),
    resolveArtifact: native.resolveArtifact,
    maxBytes: 64 * 1024,
  })
  return { dir, open, call, received, pipes, log: () => remote.log.read({ since: 0, limit: 1000 }) }
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

test(
  'the Host log keeps a stream\'s standard error under its own source, and the runner\'s words about streams and services',
  async () => {
    const { open, log } = await connected()
    const result = await open('result')
    expect(decodeUtf8(await collectBytes(result.output.stream()))).toBe('command output')
    result.stream.close()
    await open('missing').catch(() => null)

    // A read waits for no queued line: ask until the writer has put the
    // stream's end in the files.
    const ended = (line: { text: string }) => line.text === 'stream:result ended'
    let page = await log()
    for (let tries = 0; tries < 500 && !page.lines.some(ended); tries += 1) {
      await delay(10)
      page = await log()
    }
    const told = page.lines.map(({ source, conversationId, text }) => ({ source, conversationId, text }))
    const conversationId = context.conversation
    expect(told).toContainEqual({ source: 'stream:result', conversationId, text: 'command diagnostic' })
    expect(told).toContainEqual({ source: 'runner', conversationId, text: 'stream:result opened' })
    expect(told).toContainEqual({ source: 'runner', conversationId, text: 'stream:result ended' })
    expect(told).toContainEqual({
      source: 'runner',
      conversationId,
      text: 'stream:missing refused (unknown_operation): demicodes.runner-test has no operation missing',
    })
    expect(told.some(line => line.source === 'runner'
      && /^service demicodes\.runner-test started \(pid \d+\)$/.test(line.text))).toBe(true)
  },
  60_000,
)

test(
  'a one-shot call carries its arguments and learns a failed invocation\'s exit code and words',
  async () => {
    const { call, received } = await connected()
    const answer = JSON.parse(decodeUtf8(await call('where', { label: 'from the user' })))
    expect(answer.label).toBe('from the user')
    expect(answer.context.caller).toEqual({ kind: 'user' })
    expect(received.some(message => message.type === 'service_done' && message.exitCode === 0)).toBe(true)

    // `result` writes to both outputs and exits 17: the call fails with what it said on standard error.
    const failed = await call('result').then(() => null, (error: unknown) => error)
    expect(failed).toBeInstanceOf(RemoteServiceExit)
    if (!(failed instanceof RemoteServiceExit)) throw new Error('Expected a service exit')
    expect(failed.exitCode).toBe(17)
    expect(failed.stderr).toBe('command diagnostic')
    expect(decodeUtf8(failed.stdout)).toBe('command output')
  },
  60_000,
)
