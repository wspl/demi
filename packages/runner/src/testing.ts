// Test helpers for driving the tinyjs runner from Bun tests: the packed
// binary, and a runner process with its pairing code and status captured.
// Shipped as `@demicodes/runner/testing`, never bundled.
import { mkdir, realpath, symlink } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { bundleForTinyjs, tinyjsBinary } from '@demicodes/host-runner/testing'
import { waitFor } from '@demicodes/utils'

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
    await bundleForTinyjs(resolve(import.meta.dir, 'tinyjs', 'entry.ts'), bundle)
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
