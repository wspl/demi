// Test fixtures for the Rust runner and the target-independent Host contract.
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { nativePackageSchema, NATIVE_TARGETS, type ArtifactResolver } from '@demicodes/command-protocol'
import { join, resolve } from 'node:path'
import { buildNativePackageFixture } from './testing/native-package'

const repositoryRoot = resolve(import.meta.dir, '../../..')
let binary: Promise<string> | undefined
let packageFixture: ReturnType<typeof buildNativePackageFixture> | undefined

export { TEST_COMMAND_CONTEXT } from '@demicodes/shell/testing'

/**
 * The directory of the executables tests start. The test script builds them
 * first (`bun run test`); a test never builds one itself.
 */
function testPrograms(): string {
  const directory = process.env.DEMI_TEST_PROGRAMS
  if (!directory)
    throw new Error('Set DEMI_TEST_PROGRAMS to the built test programs: cargo build --workspace --all-targets --features demi-runner/test-fixtures, then DEMI_TEST_PROGRAMS=target/debug (bun run test does both)')
  return resolve(repositoryRoot, directory)
}

export function nativePackageFixture() {
  return packageFixture ??= buildNativePackageFixture(testPrograms())
}

/** The runner tests start: `DEMI_RUNNER_TEST_BINARY`, else the built test program. */
export function runnerBinary(): Promise<string> {
  return binary ??= (async () => process.env.DEMI_RUNNER_TEST_BINARY
    ? resolve(process.env.DEMI_RUNNER_TEST_BINARY)
    : join(testPrograms(), `demi-runner${process.platform === 'win32' ? '.exe' : ''}`))()
}

export interface RunnerOptions {
  backendUrl: string
  stateDir: string
  home: string
  name?: string
  deviceToken?: string
  managed?: boolean
  env?: Record<string, string>
}

export interface Runner {
  codes: string[]
  statuses: string[]
  details: string[]
  log: string[]
  exited: Promise<void>
  /** The process, for a test that pauses the Host or watches it die. */
  pid: number
  stop(): Promise<void>
}

export async function startRunner(options: RunnerOptions): Promise<Runner> {
  const bin = await runnerBinary()
  if (options.deviceToken) {
    await mkdir(options.stateDir, { recursive: true, mode: 0o700 })
    await writeFile(join(options.stateDir, 'runner-token'), `${options.deviceToken}\n`, { mode: 0o600 })
  }
  const child = Bun.spawn([bin, 'run', '--backend', options.backendUrl], {
    cwd: options.home,
    env: {
      ...process.env,
      ...options.env,
      HOME: options.home,
      USERPROFILE: options.home,
      DEMI_HOME: options.stateDir,
      ...(options.name ? { DEMI_RUNNER_NAME: options.name } : {}),
      DEMI_RUNNER_MANAGED: options.managed ? '1' : undefined,
    },
    stdout: 'pipe', stderr: 'pipe',
  })
  const codes: string[] = []
  const statuses: string[] = []
  const details: string[] = []
  const log: string[] = []
  const ready = Promise.withResolvers<void>()
  const read = async (stream: ReadableStream<Uint8Array>) => {
    const decoder = new TextDecoder()
    let buffer = ''
    const line = (value: string) => {
      log.push(value)
      const code = /^demi-runner: pairing code: (\S+)/.exec(value)
      if (code) {
        codes.push(code[1]!)
        statuses.push('claim_pending')
      } else if (value === 'demi-runner: online') {
        statuses.push('online')
      } else if (value.startsWith('demi-runner: registration refused')) {
        statuses.push(value.includes('(already_connected)') ? 'connecting' : 'rejected')
        details.push(value)
      } else if (value.startsWith('demi-runner: connection ended')) {
        statuses.push('connecting')
      }
      if (statuses.length > 0)
        ready.resolve()
    }
    for await (const chunk of stream) {
      buffer += decoder.decode(chunk, { stream: true })
      let index
      while ((index = buffer.indexOf('\n')) >= 0) {
        line(buffer.slice(0, index))
        buffer = buffer.slice(index + 1)
      }
    }
    buffer += decoder.decode()
    if (buffer)
      line(buffer)
  }
  const readers = Promise.all([read(child.stdout), read(child.stderr)])
  const exited = Promise.all([child.exited, readers]).then(() => {})
  void exited.then(() => {
    if (statuses.length === 0)
      ready.reject(new Error(`Runner exited before registration: ${log.join('\n')}`))
  }, ready.reject)
  const runner: Runner = {
    codes, statuses, details, log, exited, pid: child.pid,
    stop: async () => {
      if (child.exitCode !== null) {
        await exited
        return
      }
      child.kill('SIGTERM')
      const kill = setTimeout(() => child.kill('SIGKILL'), 5_000)
      try {
        await exited
        statuses.push('stopped')
      } finally {
        clearTimeout(kill)
      }
    },
  }
  const startup = setTimeout(() => ready.reject(new Error('Runner registration timed out')), 10_000)
  try {
    await ready.promise
    return runner
  } catch (error) {
    await runner.stop()
    throw error
  } finally {
    clearTimeout(startup)
  }
}

export { LocalHost, type LocalHostOptions } from './testing/local-host'
export { nodeFileSystem } from './testing/node-fs'

/** Host-only fault-injection service. Its descriptor must never be published. */
export async function nativeCommandFixture() {
  const path = join(await runnerBinary(), '..', `demi-native-fixture${process.platform === 'win32' ? '.exe' : ''}`)
  const bytes = await readFile(path)
  const artifact = { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length }
  const descriptor = nativePackageSchema.parse({
    id: 'demicodes.runner-test', version: 'test', protocolVersion: 1,
    operations: ['where', 'echo', 'first', 'spin', 'result', 'retain', 'crash'],
    targets: Object.fromEntries(NATIVE_TARGETS.map(target => [target, artifact])),
  })
  const resolveArtifact: ArtifactResolver = async (requested, signal) => {
    signal.throwIfAborted()
    if (requested.sha256 !== artifact.sha256 || requested.size !== artifact.size)
      throw new Error('Artifact is outside the test catalog')
    return { path }
  }
  return { descriptor, resolveArtifact }
}

export { connectTestRunner, pipeId, servePipe, TEST_RUNNER_DEVICE } from './testing/connection'
