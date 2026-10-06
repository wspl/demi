// `bun run web:dev`: generates the contracts, then runs the product's Vite dev
// server under Node with the arguments given (web-application.md § Development
// and checks). The dev server forwards the page's WebSockets to the backend,
// which Vite cannot do under Bun: Bun's `node:http` client never reports the
// backend's 101 as an upgrade (Bun 1.3.11). Every `node` a `bun run` script
// starts is Bun itself here (bunfig.toml's `[run] bun = true` puts a `node` of
// Bun first on PATH), so this takes the first `node` on PATH that is not Bun.
import { realpathSync } from 'node:fs'
import { delimiter, join, resolve } from 'node:path'
import { runSteps } from './run-steps'

const bun = realpathSync(process.execPath)

/** Whether `candidate` is a Node executable, and not Bun standing in for one. */
function isNode(candidate: string): boolean {
  try {
    return realpathSync(candidate) !== bun
  } catch {
    // A directory of PATH without a `node` offers none.
    return false
  }
}

const node = (process.env['PATH'] ?? '')
  .split(delimiter)
  .map((directory) => join(directory, 'node'))
  .find(isNode)
if (!node) {
  console.error('web:dev runs Vite under Node, and no Node is on PATH (web-application.md § Development and checks).')
  process.exit(1)
}

const repository = resolve(import.meta.dir, '..')
const web = resolve(repository, 'packages/web')
await runSteps([
  { command: [process.execPath, 'run', 'contracts'], cwd: repository },
  { command: [node, join(web, 'node_modules/vite/bin/vite.js'), ...Bun.argv.slice(2)], cwd: web },
])
