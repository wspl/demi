import { expect, test } from 'bun:test'
import { createWriteStream, existsSync } from 'node:fs'
import { mkdir, mkdtemp, open, readFile, realpath, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-local'
import { memoryHostStore } from '@demicodes/shell/testing'
import { delay, waitFor } from '@demicodes/utils'
import {
  HostRpcServer,
  JOB_VIEW_BYTES,
  JobTable,
  RemoteHost,
  RemoteShellEnvironment,
  createRunnerWire,
  type JobSpawnHandle,
  type JobSpawnParams,
} from '../index'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'

// The job table on a local Host with a JavaScript tee (the tinyjs runner
// brings the primitive): the backend's RemoteShellEnvironment drives it
// through the codec both ways.

const wire = createRunnerWire(msgpackCodec)

async function connected() {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'demi-jobs-')))
  const local = new LocalHost(dir)
  const remote = new RemoteHost({ defaultCwd: dir, commandArtifactsDir: join(dir, '.out'), identity: local.identity, store: memoryHostStore() })
  const send = (message: Parameters<typeof wire.encode>[0]) => remote.handleMessage(wire.decodeRunnerToBackend(wire.encode(message)))
  const rpc = new HostRpcServer(local, send)
  const jobs = new JobTable({
    spawn: (params) => teedSpawn(local, params),
    outputDir: join(dir, '.out'),
    fs: {
      mkdir: async (path) => {
        await mkdir(path, { recursive: true })
      },
      readTail: async (path, bytes) => {
        const size = (await stat(path)).size
        const handle = await open(path, 'r')
        try {
          const length = Math.min(size, bytes)
          const buffer = new Uint8Array(length)
          await handle.read(buffer, 0, length, size - length)
          return buffer
        } finally {
          await handle.close()
        }
      },
      readFile: (path) => readFile(path),
      rm: (path) => rm(path, { force: true }),
    },
    deviceEnv: { PATH: '/usr/bin:/bin' },
    send,
  })
  remote.attach((message) => {
    const decoded = wire.decodeBackendToRunner(wire.encode(message))
    void (decoded.type.startsWith('job_') ? jobs.handleMessage(decoded) : rpc.handleMessage(decoded))
  })
  const shell = new RemoteShellEnvironment({ host: remote, initialEnv: { PATH: '/usr/bin:/bin' } })
  return { dir, remote, jobs, shell }
}

test('a job runs bash -c on the runner; its output files are the artifact directory', async () => {
  const { dir, shell, jobs } = await connected()
  const result = await shell.exec({ script: 'echo hello; echo oops >&2; exit 4', timeoutMs: 5_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') return
  expect(result.exitCode).toBe(4)
  expect(result.stdout.delta).toBe('hello\n')
  expect(result.stderr.delta).toBe('oops\n')
  expect(result.output.chunks).toEqual([{ stream: 'stdout', text: 'hello\n' }, { stream: 'stderr', text: 'oops\n' }])
  expect(result.artifactDir.startsWith(join(dir, '.out'))).toBe(true)
  expect(await readFile(join(result.artifactDir, 'stdout.txt'), 'utf8')).toBe('hello\n')
  expect(existsSync(join(result.artifactDir, 'cwd'))).toBe(false)
  expect(jobs.count).toBe(0)
})

test('the working directory carries between jobs of a shell, an explicit exit included; env does not', async () => {
  const { dir, shell } = await connected()
  await mkdir(join(dir, 'sub'))
  const first = await shell.exec({ script: 'cd sub && export FOO=1 && pwd', timeoutMs: 5_000 })
  expect(first.status === 'exited' && first.stdout.delta).toBe(`${join(dir, 'sub')}\n`)
  const second = await shell.exec({ script: 'pwd; echo "${FOO:-unset}"', timeoutMs: 5_000 })
  expect(second.status === 'exited' && second.stdout.delta).toBe(`${join(dir, 'sub')}\nunset\n`)
  const third = await shell.exec({ script: 'cd ..; exit 3', timeoutMs: 5_000 })
  expect(third.status === 'exited' && third.exitCode).toBe(3)
  const fourth = await shell.exec({ script: 'pwd', timeoutMs: 5_000 })
  expect(fourth.status === 'exited' && fourth.stdout.delta).toBe(`${dir}\n`)
  // A script bash cannot parse never runs the trap: the directory stays.
  const broken = await shell.exec({ script: 'cd sub; do', timeoutMs: 5_000 })
  expect(broken.status === 'exited' && broken.exitCode).toBe(2)
  const after = await shell.exec({ script: 'pwd', timeoutMs: 5_000 })
  expect(after.status === 'exited' && after.stdout.delta).toBe(`${dir}\n`)
  // An ephemeral exec starts where it is told and leaves the default shell alone.
  const ephemeral = await shell.exec({ script: 'pwd', timeoutMs: 5_000, ephemeral: true, cwd: join(dir, 'sub') })
  expect(ephemeral.status === 'exited' && ephemeral.stdout.delta).toBe(`${join(dir, 'sub')}\n`)
})

test('a job outliving the timeout is running, counts in the job table, takes stdin, and can be aborted', async () => {
  const { shell, jobs } = await connected()
  const running = await shell.exec({ script: 'echo ready; head -n1; sleep 30', timeoutMs: 200 })
  expect(running.status).toBe('running')
  for (let tries = 0; (await shell.status({ commandId: running.commandId, stdoutOffset: 0 })).stdout.delta !== 'ready\n'; tries += 1) {
    if (tries > 200) throw new Error('the head of the view never arrived')
    await delay(20)
  }
  expect(jobs.count).toBe(1)
  const written = await shell.write({ commandId: running.commandId, stdin: 'typed\n' })
  expect(written.status).toBe('running')
  const aborted = await shell.abort({ commandId: running.commandId })
  expect(aborted.status === 'exited' ? aborted.exitCode : 130).toBe(130)
  await waitFor(() => jobs.count === 0, undefined, { timeoutMs: 5_000 })
  const status = await shell.status({ commandId: running.commandId })
  expect(status.status).not.toBe('running')
})

test('output beyond the view is the head, a gap note and the true tail; the full stream is in the file', async () => {
  const { shell } = await connected()
  const total = 100_000
  const result = await shell.exec({ script: `i=0; while [ $i -lt ${total / 10} ]; do printf '%09d\\n' $i; i=$((i+1)); done`, timeoutMs: 10_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') return
  const text = result.stdout.delta
  expect(text.startsWith('000000000\n000000001\n')).toBe(true)
  expect(text.endsWith(`${String(total / 10 - 1).padStart(9, '0')}\n`)).toBe(true)
  expect(text).toContain(`bytes not shown; the full stream is at ${join(result.artifactDir, 'stdout.txt')}`)
  expect(text.length).toBeLessThan(2 * JOB_VIEW_BYTES + 200)
  expect((await stat(join(result.artifactDir, 'stdout.txt'))).size).toBe(total)
})

test('a dropped connection kills the job on the runner and fails it in the backend', async () => {
  const { remote, shell, jobs } = await connected()
  const running = await shell.exec({ script: 'sleep 30', timeoutMs: 100 })
  expect(running.status).toBe('running')
  remote.detach('runner disconnected')
  await jobs.close()
  const status = await shell.status({ commandId: running.commandId })
  expect(status.status).toBe('exited')
  expect(status.status === 'exited' && status.stderr.delta).toContain('runner disconnected')
  expect(jobs.count).toBe(0)
})

/** A tee in JavaScript over a Host without the primitive: the full streams to files, the head as the view. */
async function teedSpawn(host: LocalHost, params: JobSpawnParams): Promise<JobSpawnHandle> {
  const handle = await host.process.spawn({ command: params.command, args: params.args, cwd: params.cwd, env: params.env, killProcessGroup: true })
  const counts = { stdout: 0, stderr: 0 }
  const flushed: Promise<void>[] = []
  const tee = (stream: AsyncIterable<Uint8Array>, name: 'stdout' | 'stderr', path: string) => {
    const file = createWriteStream(path)
    flushed.push(new Promise<void>((resolve) => file.once('close', () => resolve())))
    const done = (async function* () {
      let shown = 0
      try {
        for await (const chunk of stream) {
          counts[name] += chunk.byteLength
          file.write(chunk)
          if (shown < params.tee.viewLimit) {
            const part = chunk.subarray(0, params.tee.viewLimit - shown)
            shown += part.byteLength
            yield part
          }
        }
      } finally {
        file.end()
      }
    })()
    return done
  }
  const stdout = tee(handle.stdout, 'stdout', params.tee.stdoutPath)
  const stderr = tee(handle.stderr, 'stderr', params.tee.stderrPath)
  return {
    stdout,
    stderr,
    writeStdin: (data) => handle.writeStdin(data),
    closeStdin: () => handle.closeStdin(),
    kill: (signal) => handle.kill(signal),
    wait: async () => {
      const exit = await handle.wait()
      await Promise.all(flushed)
      return { ...exit, stdoutBytes: counts.stdout, stderrBytes: counts.stderr }
    },
  }
}
