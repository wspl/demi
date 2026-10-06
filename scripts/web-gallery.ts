// `bun run web:gallery`: generates the contracts, then runs the gallery's
// `dev` script with the arguments given.
import { resolve } from 'node:path'
import { runSteps } from './run-steps'

const repository = resolve(import.meta.dir, '..')

await runSteps([
  { command: [process.execPath, 'run', 'contracts'], cwd: repository },
  {
    command: [process.execPath, 'run', 'dev', ...Bun.argv.slice(2)],
    cwd: resolve(repository, 'packages/web-gallery'),
  },
])
