// Test helpers for Bun tests that run JS on txiki.js or need a runner
// process: the binaries, the bundle, the packed runner, and a runner
// process with its pairing code and status captured. Shipped as
// `@demicodes/runner/testing`, never bundled.
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { existsSync, copyFileSync } from 'node:fs'
import { commandClientBinary } from '../../command-client/build'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { waitFor } from '@demicodes/utils'

import { runtimeBinary, packRuntime } from '../runtime/build'
export const txikiBinary = runtimeBinary
export { packRuntime }

/**
 * Bundles an entry for txiki.js: one ESM file, workspace packages from their
 * sources, `tjs:*` left to the runtime. Runs the bundler in its own
 * process: an in-process `Bun.build` for a browser target leaves the test
 * process unable to resolve some of the same packages afterwards.
 */
export async function bundleForTxiki(
  entry: string,
  outfile: string
): Promise<void> {
  const built = Bun.spawnSync(
    ['bun', resolve(import.meta.dir, '../runtime/bundle.ts'), entry, outfile],
    { stdout: 'pipe', stderr: 'pipe' },
  )
  if (!built.success)
    throw new Error(
      `bundle failed:\n${built.stderr.toString()}${built.stdout.toString()}`
    )
}

let packed: Promise<string> | null = null

/**
 * The packed txiki.js runner with the native `demi` client beside it, built
 * once per test process.
 */
export function packedRunner(): Promise<string> {
  return (packed ??= (async () => {
    const cache = resolve(import.meta.dir, '../../..', '.cache/txiki/runners')
    await mkdir(cache, { recursive: true })
    const bundle = join(cache, `entry-${process.pid}.mjs`)
    await bundleForTxiki(resolve(import.meta.dir, 'entry.ts'), bundle)
    const runtime = runtimeBinary()
    const client = commandClientBinary()
    const hash = createHash('sha256')
    for (const path of [
      bundle,
      runtime,
      client,
      resolve(import.meta.dir, '../runtime/build.ts'),
      resolve(import.meta.dir, '../../../vendor/txiki.js/src/cli.c'),
      resolve(import.meta.dir, '../../../vendor/txiki.js/CMakeLists.txt')
    ]) {
      hash.update(await readFile(path))
    }
    const work = join(cache, hash.digest('hex'))
    const file = join(work, 'demi-runner')
    if (!existsSync(file)) {
      await mkdir(work, { recursive: true })
      packRuntime(bundle, file)
    }
    copyFileSync(client, join(work, 'demi'))
    return join(work, 'demi-runner')
  })())
}

export interface TxikiRunnerOptions {
  backendUrl: string
  /**
   * `DEMI_HOME`: runner.json, runner-token, active.json, runner.lock, commands,
   * output.
   */
  stateDir: string
  /** `HOME` inside the runner: its default working directory. */
  home: string
  name?: string
  /**
   * A pre-issued device token written to the state directory before the start —
   * how a managed host joins in tests.
   */
  deviceToken?: string
  /** Start as a managed host: the hello carries `managed: true`. */
  managed?: boolean
}

export interface TxikiRunner {
  codes: string[]
  statuses: string[]
  details: string[]
  /** Everything the runner printed. */
  log: string[]
  /** Resolves when the process has exited, however it did. */
  exited: Promise<void>
  stop(): Promise<void>
}

/** Starts `demi-runner run --backend <url>` and captures its lines. */
export async function startTxikiRunner(
  options: TxikiRunnerOptions
): Promise<TxikiRunner> {
  const bin = await packedRunner()
  if (options.deviceToken) {
    await mkdir(options.stateDir, { recursive: true })
    await writeFile(
      join(options.stateDir, 'runner-token'),
      `${options.deviceToken}\n`,
      { mode: 0o600 }
    )
  }
  const child = Bun.spawn([bin, 'run', '--backend', options.backendUrl], {
    env: {
      PATH: process.env.PATH ?? '/usr/bin:/bin',
      HOME: options.home,
      DEMI_HOME: options.stateDir,
      DEMI_RUNNER_RECONNECT_MS: '30',
      ...(options.name ? { DEMI_RUNNER_NAME: options.name } : {}),
      ...(options.managed ? { DEMI_RUNNER_MANAGED: '1' } : {}),
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const runner: TxikiRunner = {
    codes: [],
    statuses: [],
    details: [],
    log: [],
    exited: child.exited.then(() => {}),
    stop: async () => {}
  }
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
          runner.statuses.push(line.includes('already_connected')
            ? 'connecting'
            : 'rejected')
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
  await waitFor(
    () => runner.statuses.length > 0,
    undefined,
    { timeoutMs: 10_000 }
  )
  return runner
}

export { LocalHost, type LocalHostOptions } from './testing/local-host'
export { nodeFileSystem } from './testing/node-fs'
