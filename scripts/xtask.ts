// `bun xtask <command>` (builds-and-releases.md): builds the one Cargo
// selection, which holds xtask, and runs target/debug/xtask with the arguments
// given. Bun reads the repository's .env into this process's environment but
// not into a package.json script's, so this passes it on: the
// DEMI_DEV_PROVIDER_* variables reach `xtask dev`.
import { resolve } from 'node:path'

const repository = resolve(import.meta.dir, '..')

let child: Bun.Subprocess | undefined
// An interrupt or a stop reaches the running step, which stops what it
// started; this ends when it does.
for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => child?.kill(signal))
}

async function run(command: string[]): Promise<number> {
  child = Bun.spawn(command, {
    cwd: repository,
    env: process.env,
    stdio: ['inherit', 'inherit', 'inherit'],
  })
  return await child.exited
}

const built = await run([
  'cargo',
  'build',
  '--workspace',
  '--all-targets',
  '--features',
  'demi-runner/test-fixtures',
])
if (built !== 0) {
  process.exit(built)
}
process.exit(await run([resolve(repository, 'target/debug/xtask'), ...Bun.argv.slice(2)]))
