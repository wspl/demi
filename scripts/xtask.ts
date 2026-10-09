// `bun xtask <command>` (builds-and-releases.md): builds the one Cargo
// selection, which holds xtask, and runs target/debug/xtask with the arguments
// given. xtask runs detached: it ends by itself when this program ends, and
// `xtask dev` then stops the backend and removes its data directory, which
// a SIGKILL to this program's process group would leave behind.
import { resolve } from 'node:path'
import { runSteps } from './run-steps'

const repository = resolve(import.meta.dir, '..')

await runSteps([
  {
    command: ['cargo', 'build', '--workspace', '--all-targets', '--features', 'demi-runner/test-fixtures'],
    cwd: repository,
  },
  { command: [resolve(repository, 'target/debug/xtask'), ...Bun.argv.slice(2)], cwd: repository, detached: true },
])
