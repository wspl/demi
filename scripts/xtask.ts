// `bun xtask <command>` (`crates/xtask/src/native/build.rs`): builds the one
// Cargo selection, which holds xtask, and runs target/debug/xtask with the
// arguments given. xtask runs detached: it ends by itself when this program
// ends, and `xtask dev` then stops the backend and removes its data directory,
// which a SIGKILL to this program's process group would leave behind.
import { delimiter, dirname, resolve } from 'node:path'
import { findCargo } from './cargo'
import { runSteps } from './run-steps'

const repository = resolve(import.meta.dir, '..')

let cargo: string
try {
  cargo = findCargo()
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  process.exit(1)
}
// xtask runs `cargo` and `rustc` by name, and rustup installs both proxies in
// one directory: with that directory on PATH, xtask finds the ones found here.
process.env.PATH = [dirname(cargo), process.env.PATH].filter(Boolean).join(delimiter)

await runSteps([
  {
    command: [cargo, 'build', '--workspace', '--all-targets', '--features', 'demi-runner/test-fixtures'],
    cwd: repository,
  },
  { command: [resolve(repository, 'target/debug/xtask'), ...Bun.argv.slice(2)], cwd: repository, detached: true },
])
