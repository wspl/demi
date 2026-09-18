import { afterEach, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, realpath, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { connectTestRunner, TEST_RUNNER_DEVICE } from '@demicodes/host-remote/testing'
import { memoryHostStore } from '@demicodes/shell/testing'
import { delay, waitFor } from '@demicodes/utils'
import { PipeBroker, RemoteHost, RemoteShellEnvironment, devicePipes } from '@demicodes/host-remote'
import { JOB_VIEW_BYTES } from '@demicodes/runner-protocol'

const cleanups: Array<() => Promise<void>> = []
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup()
})

async function connected() {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'demi-jobs-')))
  cleanups.push(() => rm(dir, { recursive: true, force: true }))
  let remote!: RemoteHost
  const pipes = new PipeBroker()
  const connection = await connectTestRunner({
    home: dir,
    pipes,
    env: { DEVICE_FACT: 'from the device', SHARED: 'device' },
    onHello(send, hello) {
      remote = new RemoteHost({
        defaultCwd: dir,
        identity: hello.runner.identity,
        store: memoryHostStore(),
        pipes: devicePipes(pipes, TEST_RUNNER_DEVICE),
      })
      remote.attach(send)
    },
    onMessage: message => remote.handleMessage(message),
    onClose: () => remote.detach('runner disconnected'),
  })
  cleanups.push(connection.close)
  const shell = new RemoteShellEnvironment({ conversation: 'test-conversation', node: 'test-session',
    host: remote,
    initialEnv: { PATH: '/usr/bin:/bin' }
  })
  cleanups.push(() => shell.disposeAllShells())
  return { dir, remote, connection, shell }
}

test(
  'a job runs in the embedded shell on the runner; its output files are the artifact directory',
  async () => {
    const { dir, shell, remote } = await connected()
    const result = await shell.exec({
      script: 'echo hello; echo oops >&2; exit 4',
      timeoutMs: 5_000
    })
    expect(result.status).toBe('exited')
    if (result.status !== 'exited')
      return
    expect(result.exitCode).toBe(4)
    expect(result.stdout.delta).toBe('hello\n')
    expect(result.stderr.delta).toBe('oops\n')
    // Independent OS pipes preserve each stream's bytes, not inter-stream order.
    for (const [stream, text] of [['stdout', 'hello\n'], ['stderr', 'oops\n']]) {
      expect(result.output.chunks.filter(chunk => chunk.stream === stream)
        .map(chunk => chunk.text).join('')).toBe(text)
    }
    expect(result.outputDir).toBeDefined()
    expect(result.stdout.path).toBe(join(result.outputDir!, 'stdout.txt'))
    expect(await readFile(join(result.outputDir!, 'stdout.txt'), 'utf8'))
      .toBe('hello\n')
    expect(existsSync(join(result.outputDir!, 'cwd'))).toBe(false)
    expect(remote.activeJobCount).toBe(0)
  }
)

test(
  'a job runs in the device environment underneath the shell\'s: a device entry reaches it, a backend entry wins, DEMI_HOME is the runner\'s',
  async () => {
    const { shell } = await connected()
    const result = await shell.exec({
      script: 'echo "$DEVICE_FACT|$SHARED|${DEMI_SESSION_ID:-none}"; echo "$PATH"',
      timeoutMs: 5_000,
      agentSessionId: 's1'
    })
    const [facts, path] = (result.status === 'exited' ? result.stdout.delta : '').split('\n')
    expect(facts).toBe('from the device|device|s1')
    expect(path?.split(':')).toContain('/usr/bin')
    const overriding = new RemoteShellEnvironment({ conversation: 'test-conversation', node: 'test-session',
      host: (await connected()).remote,
      initialEnv: { SHARED: 'backend' }
    })
    const overridden = await overriding.exec({
      script: 'echo "$SHARED"',
      timeoutMs: 5_000
    })
    expect(overridden.status === 'exited' && overridden.stdout.delta)
      .toBe('backend\n')
  }
)

test(
  'the working directory carries between jobs of a shell, an explicit exit included; env does not',
  async () => {
    const { dir, shell } = await connected()
    await mkdir(join(dir, 'sub'))
    const first = await shell.exec({
      script: 'cd sub && export FOO=1 && pwd',
      timeoutMs: 5_000
    })
    expect(first.status === 'exited' && first.stdout.delta)
      .toBe(`${join(dir, 'sub')}\n`)
    const second = await shell.exec({
      script: 'pwd; echo "${FOO:-unset}"',
      timeoutMs: 5_000
    })
    expect(second.status === 'exited' && second.stdout.delta)
      .toBe(`${join(dir, 'sub')}\nunset\n`)
    const third = await shell.exec({ script: 'cd ..; exit 3', timeoutMs: 5_000 })
    expect(third.status === 'exited' && third.exitCode).toBe(3)
    const fourth = await shell.exec({ script: 'pwd', timeoutMs: 5_000 })
    expect(fourth.status === 'exited' && fourth.stdout.delta).toBe(`${dir}\n`)
    // A parse error leaves the working directory unchanged.
    const broken = await shell.exec({ script: 'cd sub; do', timeoutMs: 5_000 })
    expect(broken.status === 'exited' && broken.exitCode).toBe(2)
    const after = await shell.exec({ script: 'pwd', timeoutMs: 5_000 })
    expect(after.status === 'exited' && after.stdout.delta).toBe(`${dir}\n`)
    // An ephemeral exec starts where it is told and leaves the default shell alone.
    const ephemeral = await shell.exec({
      script: 'pwd',
      timeoutMs: 5_000,
      ephemeral: true,
      cwd: join(dir, 'sub')
    })
    expect(ephemeral.status === 'exited' && ephemeral.stdout.delta)
      .toBe(`${join(dir, 'sub')}\n`)
  }
)

test(
  'a job outliving the timeout is running, counts in the job table, takes stdin, and can be aborted',
  async () => {
    const { shell, remote } = await connected()
    const running = await shell.exec({
      script: 'echo ready; head -n1; sleep 30',
      timeoutMs: 200
    })
    expect(running.status).toBe('running')
    for (let tries = 0; (await shell.status({ commandId: running.commandId })).stdout.tail !== 'ready\n'; tries += 1) {
      if (tries > 200)
        throw new Error('the head of the view never arrived')
      await delay(20)
    }
    expect(remote.activeJobCount).toBe(1)
    const written = await shell.write({
      commandId: running.commandId,
      stdin: 'typed\n'
    })
    expect(written.status).toBe('running')
    const aborted = await shell.abort({ commandId: running.commandId })
    expect(aborted.status === 'exited' ? aborted.exitCode : 130).toBe(130)
    await waitFor(() => remote.activeJobCount === 0, undefined, { timeoutMs: 5_000 })
    const status = await shell.status({ commandId: running.commandId })
    expect(status.status).not.toBe('running')
  }
)

test(
  'output beyond the view is the head, a gap note and the true tail; the full stream is in the file',
  async () => {
    const { shell } = await connected()
    const total = 100_000
    const result = await shell.exec({
      script: `seq -f '%09g' 0 ${total / 10 - 1}`,
      timeoutMs: 10_000
    })
    expect(result.status).toBe('exited')
    if (result.status !== 'exited')
      return
    const text = result.stdout.delta
    expect(text.startsWith('000000000\n000000001\n')).toBe(true)
    expect(text.endsWith(`${String(total / 10 - 1).padStart(9, '0')}\n`))
      .toBe(true)
    expect(text).toContain(
      `bytes not shown; the full stream is at ${join(result.outputDir!, 'stdout.txt')}`
    )
    expect(text.length).toBeLessThan(2 * JOB_VIEW_BYTES + 200)
    expect((await stat(join(result.outputDir!, 'stdout.txt'))).size).toBe(total)
  }
)

test(
  'a dropped connection kills the job on the runner and fails it in the backend',
  async () => {
    const { remote, shell, connection } = await connected()
    const running = await shell.exec({ script: 'sleep 30', timeoutMs: 100 })
    expect(running.status).toBe('running')
    remote.detach('runner disconnected')
    await connection.close()
    const status = await shell.status({ commandId: running.commandId })
    expect(status.status).toBe('exited')
    expect(status.status === 'exited' && status.stderr.delta)
      .toContain('runner disconnected')
    expect(remote.activeJobCount).toBe(0)
  }
)
