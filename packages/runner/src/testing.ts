// Test helpers for Bun tests that run JS on tinyjs or need a runner
// process: the binaries, the bundle, the packed runner, and a runner
// process with its pairing code and status captured. Shipped as
// `@demicodes/runner/testing`, never bundled. The runner's protocol end
// (`HostRpcServer`) is re-exported for tests that join it to a
// `RemoteHost` without a socket.
import { existsSync } from 'node:fs'
import { mkdir, realpath, symlink } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { waitFor } from '@demicodes/utils'

export { HostRpcServer } from './serve/host-rpc-server'

const CRATE = resolve(import.meta.dir, '..', '..', 'tinyjs')

/**
 * The path of a tinyjs binary, building the crate in debug mode when it is
 * missing. `TINYJS_BIN` names a prebuilt `tinyjs` instead.
 */
export function tinyjsBinary(name: 'tinyjs' | 'tinyjsc' = 'tinyjs'): string {
  if (name === 'tinyjs' && process.env.TINYJS_BIN) return process.env.TINYJS_BIN
  const path = join(CRATE, 'target', 'debug', name)
  if (existsSync(path)) return path
  const home = process.env.HOME ?? ''
  const built = Bun.spawnSync(['cargo', 'build', '--bin', name], {
    cwd: CRATE,
    env: { ...process.env, PATH: `${process.env.PATH ?? ''}:/opt/homebrew/opt/rustup/bin:${home}/.cargo/bin` },
    stdout: 'inherit',
    stderr: 'inherit',
  })
  if (!built.success) throw new Error(`cargo build --bin ${name} failed in ${CRATE}`)
  return path
}

/**
 * Bundles an entry for tinyjs: one ESM file, workspace packages from their
 * sources, `tinyjs:*` left to the runtime. Runs the bundler in its own
 * process: an in-process `Bun.build` for a browser target leaves the test
 * process unable to resolve some of the same packages afterwards.
 */
export async function bundleForTinyjs(entry: string, outfile: string): Promise<void> {
  const built = Bun.spawnSync(
    ['bun', 'build', entry, '--format=esm', '--target=browser', '--conditions=development', '--external', 'tinyjs:*', '--outfile', outfile],
    { stdout: 'pipe', stderr: 'pipe' },
  )
  if (!built.success) throw new Error(`bundle failed:\n${built.stderr.toString()}${built.stdout.toString()}`)
}

let packed: Promise<string> | null = null

/** The packed tinyjs bundle with `demi-runner` beside it, built once per test process. */
export function packedRunner(): Promise<string> {
  return (packed ??= (async () => {
    const work = await realpath(join(tmpdir(), `demi-packed-${process.pid}`))
      .catch(async () => {
        await mkdir(join(tmpdir(), `demi-packed-${process.pid}`), { recursive: true })
        return realpath(join(tmpdir(), `demi-packed-${process.pid}`))
      })
    const bundle = join(work, 'entry.mjs')
    await bundleForTinyjs(resolve(import.meta.dir, 'entry.ts'), bundle)
    const file = join(work, 'demi-cli')
    const pack = Bun.spawnSync([tinyjsBinary('tinyjsc'), bundle, '--bin', tinyjsBinary(), '--out', file], { stdout: 'pipe', stderr: 'pipe' })
    if (pack.exitCode !== 0) throw new Error(`tinyjsc failed: ${pack.stderr.toString()}`)
    await symlink(file, join(work, 'demi-runner')).catch(() => {})
    return join(work, 'demi-runner')
  })())
}

export interface TinyjsRunnerOptions {
  backendUrl: string
  /** `DEMI_HOME`: runner.json, runner-token, runner.sock, commands, bin, output. */
  stateDir: string
  /** `HOME` inside the runner: its default working directory. */
  home: string
  name?: string
}

export interface TinyjsRunner {
  codes: string[]
  statuses: string[]
  details: string[]
  /** Everything the runner printed. */
  log: string[]
  stop(): Promise<void>
}

/** Starts `demi-runner run --backend <url>` and captures its lines. */
export async function startTinyjsRunner(options: TinyjsRunnerOptions): Promise<TinyjsRunner> {
  const bin = await packedRunner()
  const runner: TinyjsRunner = { codes: [], statuses: [], details: [], log: [], stop: async () => {} }
  const child = Bun.spawn([bin, 'run', '--backend', options.backendUrl], {
    env: {
      PATH: process.env.PATH ?? '/usr/bin:/bin',
      HOME: options.home,
      DEMI_HOME: options.stateDir,
      DEMI_RUNNER_RECONNECT_MS: '30',
      ...(options.name ? { DEMI_RUNNER_NAME: options.name } : {}),
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const read = async (stream: ReadableStream<Uint8Array>) => {
    let buffer = ''
    for await (const chunk of stream) {
      buffer += new TextDecoder().decode(chunk)
      let index
      while ((index = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, index)
        buffer = buffer.slice(index + 1)
        runner.log.push(line)
        const code = /^Pairing code: (\S+)/.exec(line)
        if (code) {
          runner.codes.push(code[1]!)
          runner.statuses.push('claim_pending')
        } else if (line === 'runner online') runner.statuses.push('online')
        else if (line.startsWith('connecting')) runner.statuses.push('connecting')
        else if (line.startsWith('refused by the backend')) {
          runner.statuses.push(line.includes('already_connected') ? 'connecting' : 'rejected')
          runner.details.push(line)
        }
      }
    }
  }
  void read(child.stderr as ReadableStream<Uint8Array>)
  void read(child.stdout as ReadableStream<Uint8Array>)
  runner.stop = async () => {
    child.kill('SIGTERM')
    await child.exited
    runner.statuses.push('stopped')
  }
  await waitFor(() => runner.statuses.length > 0, undefined, { timeoutMs: 10_000 })
  return runner
}
