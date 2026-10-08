// `bun xtask <command>` (builds-and-releases.md): builds the one Cargo
// selection, which holds xtask, and runs target/debug/xtask with the arguments
// given.
import { resolve } from 'node:path'
import { runSteps } from './run-steps'

const repository = resolve(import.meta.dir, '..')
// The development selection always carries Cargo.toml's version. A
// DEMI_WORKSPACE_VERSION that `xtask deploy` names for its web build is for
// xtask itself (`xtask preview-runtime`); in this build it would rebuild every
// crate that depends on shared-artifacts, and again the next time without it.
const { DEMI_WORKSPACE_VERSION: _deployed, ...selection } = process.env

await runSteps([
  {
    command: ['cargo', 'build', '--workspace', '--all-targets', '--features', 'demi-runner/test-fixtures'],
    cwd: repository,
    env: selection,
  },
  { command: [resolve(repository, 'target/debug/xtask'), ...Bun.argv.slice(2)], cwd: repository },
])
