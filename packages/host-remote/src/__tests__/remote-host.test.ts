import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, expect, test } from 'bun:test'
import { connectTestRunner, TEST_RUNNER_DEVICE, TEST_COMMAND_CONTEXT } from '@demicodes/host-remote/testing'
import {
  STDIN_CHUNK_BYTES,
  type BackendToRunnerMessage,
} from '@demicodes/runner-protocol'
import { memoryHostStore } from '@demicodes/shell/testing'
import { PipeBroker, RemoteGitError, RemoteHost, devicePipes } from '../index'
import { collectBytes, concatBytes, deferred } from '@demicodes/utils'

const cleanup: (() => Promise<void>)[] = []
afterEach(async () => {
  for (const close of cleanup.splice(0).reverse()) await close()
})

async function connectedPair(outgoingGate?: Promise<void>) {
  const dir = await mkdtemp(join(tmpdir(), 'demi-runner-proto-'))
  cleanup.push(() => rm(dir, { recursive: true, force: true }))
  const pipes = new PipeBroker()
  const remote = new RemoteHost({
    defaultCwd: dir,
    identity: { uid: 501, gid: 20, hostname: 'test', homeDir: '/' },
    store: memoryHostStore(),
    pipes: devicePipes(pipes, TEST_RUNNER_DEVICE),
  })
  const connect = async () => {
    const server = await connectTestRunner({
      home: dir, outgoingGate, pipes,
      onHello: send => remote.attach(send),
      onMessage: message => remote.handleMessage(message),
      onClose: () => remote.detach(),
    })
    cleanup.push(server.close)
    return server
  }
  return { dir, remote, server: await connect(), reconnect: connect }
}

test(
  'attach brings the runner identity to a Host made while its runner was offline',
  () => {
    const host = new RemoteHost({
      defaultCwd: '/w',
      identity: { uid: 0, gid: 0, hostname: 'offline', homeDir: '/w' },
      store: memoryHostStore(),
      pipes: devicePipes(new PipeBroker(), 'unused'),
    })
    host.attach(
      () => {},
      { uid: 501, gid: 20, hostname: 'laptop', homeDir: '/Users/me' }
    )
    expect(host.identity).toEqual({
      uid: 501,
      gid: 20,
      hostname: 'laptop',
      homeDir: '/Users/me'
    })
  }
)

test(
  'remote fs calls execute on the served Host and preserve error codes',
  async () => {
    const { dir, remote } = await connectedPair()

    await remote.fs.writeFile(
      join(dir, 'hello.txt'),
      new TextEncoder().encode('hi'),
      { createParents: true }
    )
    expect(new TextDecoder().decode(await remote.fs.readFile(join(
      dir,
      'hello.txt'
    )))).toBe('hi')

    const stat = await remote.fs.stat(join(dir, 'hello.txt'))
    expect(stat.isFile).toBe(true)
    expect(stat.size).toBe(2)
    expect(stat.mtime).toBeInstanceOf(Date)

    await remote.fs.mkdir(join(dir, 'sub/deeper'), { recursive: true })
    const names = await remote.fs.readdir(dir)
    expect(names.sort()).toEqual(['hello.txt', 'sub'])
    const entries = await remote.fs.readdir(dir, { withFileTypes: true })
    expect(entries.find((entry) => entry.name === 'sub')?.isDirectory)
      .toBe(true)

    expect(await remote.fs.exists(join(dir, 'nope'))).toBe(false)
    const missing = await remote.fs.readFile(join(dir, 'nope')).catch((
      error: NodeJS.ErrnoException
    ) => error)
    expect(missing).toBeInstanceOf(Error)
    expect((missing as NodeJS.ErrnoException).code).toBe('ENOENT')
  }
)

test(
  'an fs reply is read as the operation the caller asked for, not the one it names',
  async () => {
    const remote = new RemoteHost({
      defaultCwd: '/w',
      identity: { uid: 0, gid: 0, hostname: 'test', homeDir: '/w' },
      store: memoryHostStore(),
      pipes: devicePipes(new PipeBroker(), 'unused'),
    })
    const sent: BackendToRunnerMessage[] = []
    remote.attach((message) => sent.push(message))
    const stat = remote.fs.stat('/w/file')
    const call = sent.find((message) => message.type === 'fs_stat')
    expect(call).toBeDefined()
    // A `readlink` result is well formed for the op the reply names itself and
    // is not the file stat this caller is waiting for.
    remote.handleMessage({
      type: 'fs_ok',
      id: call!.id,
      op: 'readlink',
      result: '/w/elsewhere',
    })
    expect(await stat.then(() => null, (error: unknown) => error))
      .toBeInstanceOf(Error)
  }
)

test(
  'remote spawn streams output, accepts stdin, and reports exit',
  async () => {
    const { dir, remote } = await connectedPair()

    const echo = await remote.process.spawn!({
      command: '/bin/echo',
      args: ['over the wire'],
      cwd: dir,
      env: { PATH: '/usr/bin:/bin' }
    })
    expect(await collect(echo.stdout)).toBe('over the wire\n')
    expect((await echo.wait()).exitCode).toBe(0)

    const cat = await remote.process.spawn!({
      command: '/bin/cat',
      cwd: dir,
      env: { PATH: '/usr/bin:/bin' }
    })
    await cat.writeStdin(new TextEncoder().encode('stdin data'))
    await cat.closeStdin()
    expect(await collect(cat.stdout)).toBe('stdin data')
    expect((await cat.wait()).exitCode).toBe(0)

    const sleeper = await remote.process.spawn!({
      command: '/bin/sleep',
      args: ['30'],
      cwd: dir,
      env: { PATH: '/usr/bin:/bin' }
    })
    await sleeper.kill('SIGTERM')
    const exit = await sleeper.wait()
    expect(exit.exitCode).not.toBe(0)

    const missing = await remote.process.spawn!({
      command: 'not-a-real-binary',
      cwd: dir,
      env: { PATH: '/usr/bin:/bin' }
    })
    expect((await missing.wait()).spawnError?.kind).toBe('executable_not_found')
  }
)

test(
  'detach fails pending calls and kills in-flight spawn views; reattach resumes',
  async () => {
    const { dir, remote, server, reconnect } = await connectedPair()

    const running = await remote.process.spawn!({
      command: '/bin/sleep',
      args: ['30'],
      cwd: dir,
      env: { PATH: '/usr/bin:/bin' }
    })
    remote.detach('runner disconnected')
    const exit = await running.wait()
    expect(exit.exitCode).toBeNull()
    expect(exit.spawnError?.kind).toBe('other')
    await server.close() // the real connection-drop path also kills runner-side children

    await expect(remote.fs.readFile(join(dir, 'x')))
      .rejects.toThrow('runner disconnected')
    const offlineSpawn = await remote.process.spawn!({
      command: '/bin/echo',
      cwd: dir
    })
    expect((await offlineSpawn.wait()).spawnError?.kind).toBe('other')

    // Reattach: the same Host object serves again.
    await reconnect()
    await remote.fs.writeFile(
      join(dir, 'back.txt'),
      new TextEncoder().encode('online')
    )
    expect(
      new TextDecoder().decode(await remote.fs.readFile(join(dir, 'back.txt')))
    ).toBe('online')
  }
)

test('logical cwd and identity are backend-local', async () => {
  const { dir, remote } = await connectedPair()
  await remote.fs.mkdir(join(dir, 'sub'))
  const cwd = await remote.process.openCwd(dir)
  await cwd.chdir('sub')
  expect(cwd.path).toBe(join(dir, 'sub'))
  const snapshot = await cwd.snapshot()
  await cwd.chdir('..')
  snapshot.restore()
  expect(cwd.path).toBe(join(dir, 'sub'))
  await cwd.close()
  expect(remote.identity.hostname).toBe('test')
})

async function collect(stream: AsyncIterable<Uint8Array>): Promise<string> {
  let text = ''
  const decoder = new TextDecoder()
  for await (const chunk of stream) text += decoder.decode(
    chunk,
    { stream: true }
  )
  return text + decoder.decode()
}

test(
  'machine admission covers pending filesystem calls and process lifetime, then refuses cached Hosts during a transition',
  async () => {
    let active = 0
    let blocked = false
    const messages: BackendToRunnerMessage[] = []
    const host = new RemoteHost({
      defaultCwd: '/work', identity: {
        uid: 1000,
        gid: 1000,
        hostname: 'cloud',
        homeDir: '/home/demi'
      }, store: memoryHostStore(),
      pipes: devicePipes(new PipeBroker(), 'unused'),
      admit: () => {
        if (blocked)
          throw new Error('machine transition')
        active++
        return () => {
          active--
        }
      },
    })
    host.attach(message => messages.push(message))
    const exists = host.fs.exists('/work/file')
    const request = messages[0]!
    expect(request.type).toBe('fs_exists')
    expect(active).toBe(1)
    if (request.type !== 'fs_exists')
      throw new Error('Expected an exists request')
    host.handleMessage({
      type: 'fs_ok',
      id: request.id,
      op: 'exists',
      result: true
    })
    await exists
    expect(active).toBe(0)
    const process = await host.process.spawn({ command: 'sleep', args: ['10'] })
    const job = host.startJob({ context: TEST_COMMAND_CONTEXT, script: 'sleep 10', cwd: '/work', env: {} })
    expect(active).toBe(2)
    blocked = true
    await expect(host.fs.readFile('/work/next'))
      .rejects.toThrow('machine transition')
    await expect(host.process.spawn({ command: 'true' }))
      .rejects.toThrow('machine transition')
    expect(
      () => host.startJob({ context: TEST_COMMAND_CONTEXT, script: 'true', cwd: '/work', env: {} })
    ).toThrow('machine transition')
    expect(messages).toHaveLength(3)
    host.detach()
    await Promise.all([process.wait(), job.wait()])
    expect(active).toBe(0)
  }
)


test(
  'process and shell stdin writes use bounded frames and preserve bytes before EOF',
  async () => {
    const remote = new RemoteHost({
      defaultCwd: '/work',
      identity: { uid: 501, gid: 20, hostname: 'test', homeDir: '/work' },
      store: memoryHostStore(),
      pipes: devicePipes(new PipeBroker(), 'unused'),
    })
    const messages: BackendToRunnerMessage[] = []
    remote.attach(message => messages.push(message))
    try {
      const process = await remote.process.spawn({ command: 'cat' })
      const job = remote.startJob({ context: TEST_COMMAND_CONTEXT, script: 'cat', cwd: '/work', env: {} })
      for (const handle of [process, job]) {
        for (const size of [0, STDIN_CHUNK_BYTES, STDIN_CHUNK_BYTES + 1]) {
          messages.length = 0
          const bytes = Uint8Array.from({ length: size }, (_, index) => index % 256)
          await handle.writeStdin(bytes)
          const frames = messages.filter(
            message => message.type === 'spawn_stdin' || message.type === 'job_stdin'
          )
          expect(frames).toHaveLength(Math.ceil(size / STDIN_CHUNK_BYTES))
          expect(frames.every(frame => frame.bytes.byteLength <= STDIN_CHUNK_BYTES)).toBe(true)
          expect(concatBytes(frames.map(frame => frame.bytes))).toEqual(bytes)
          expect(messages).toHaveLength(frames.length)
        }
        await handle.closeStdin()
        expect(messages.at(-1)?.type).toBe(handle === process ? 'spawn_stdin_end' : 'job_stdin_end')
      }
    } finally {
      remote.detach()
    }
  }
)

test(
  'a 65,537-byte stdin write reaches cat without disconnecting the runner',
  async () => {
    const { remote } = await connectedPair()
    const bytes = Uint8Array.from(
      { length: STDIN_CHUNK_BYTES + 1 },
      (_, index) => index % 256,
    )
    const cat = await remote.process.spawn({ command: '/bin/cat' })
    await cat.writeStdin(bytes)
    await cat.closeStdin()
    expect(await collectBytes(cat.stdout)).toEqual(bytes)
    expect((await cat.wait()).exitCode).toBe(0)
    expect(await remote.fs.exists(remote.defaultCwd)).toBe(true)
  }
)

test(
  'stdin and cancellation sent during native process startup reach the new process',
  async () => {
    const ready = deferred<void>()
    const { remote, server } = await connectedPair(ready.promise)
    try {
      const cat = await remote.process.spawn({ command: '/bin/cat' })
      await cat.writeStdin(new TextEncoder().encode('early input'))
      await cat.closeStdin()
      const sleeper = await remote.process.spawn({
        command: '/bin/sleep',
        args: ['10']
      })
      await sleeper.kill('SIGKILL')
      ready.resolve()
      expect(await collect(cat.stdout)).toBe('early input')
      expect((await cat.wait()).exitCode).toBe(0)
      expect((await sleeper.wait()).signal).toBe('SIGKILL')
    } finally {
      ready.resolve();
      remote.detach();
      await server.close()
    }
  }
)

test(
  'the working-tree facet lists uncommitted changes and shows the last commit',
  async () => {
    const { dir, remote } = await connectedPair()
    const git = (...args: string[]) => {
      const result = Bun.spawnSync(['git', ...args], {
        cwd: dir,
        env: {
          ...process.env,
          GIT_AUTHOR_NAME: 'Test',
          GIT_AUTHOR_EMAIL: 'test@example.com',
          GIT_COMMITTER_NAME: 'Test',
          GIT_COMMITTER_EMAIL: 'test@example.com',
        },
      })
      expect(result.exitCode).toBe(0)
    }
    const outside = await remote.git.changes(dir)
    expect(outside).toEqual({ repository: false, head: null, files: [], truncated: false, watched: false })

    git('init', '-q', '-b', 'main')
    await Bun.write(join(dir, 'a.txt'), '1\n2\n')
    git('add', '.')
    git('commit', '-q', '-m', 'first')
    await Bun.write(join(dir, 'a.txt'), '1\n2\n3\n')
    await Bun.write(join(dir, 'b.txt'), 'new\n')

    const changes = await remote.git.changes(dir)
    expect(changes.repository).toBe(true)
    expect(changes.head).toMatch(/^[0-9a-f]{40}$/)
    expect(changes.files).toEqual([
      { path: 'a.txt', status: ' M', kind: 'modified', added: 1, removed: 0 },
      { path: 'b.txt', status: '??', kind: 'added', added: 1, removed: 0 },
    ])
    expect(new TextDecoder().decode(await remote.git.show(dir, 'a.txt'))).toBe('1\n2\n')
    const missing = await remote.git.show(dir, 'b.txt').catch((error: unknown) => error)
    expect(missing).toBeInstanceOf(RemoteGitError)
    expect((missing as RemoteGitError).code).toBe('ENOENT')
  }
)
