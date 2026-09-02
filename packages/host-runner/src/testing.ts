// Test helpers for running JS on tinyjs from Bun tests: the binaries and the
// bundle. Shipped as `@demicodes/host-runner/testing`, never bundled.
import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'

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
