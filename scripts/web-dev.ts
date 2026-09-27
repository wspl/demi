// Runs the product's Vite dev server under Node, with the arguments given
// (web-application.md § Development and checks). The dev server forwards the
// page's WebSockets to the backend, which Vite cannot do under Bun: Bun's
// `node:http` client never reports the backend's 101 as an upgrade (Bun
// 1.3.11). Every `node` a `bun run` script starts is Bun itself here
// (bunfig.toml's `[run] bun = true` puts a `node` of Bun first on PATH), so
// this takes the first `node` on PATH that is not Bun.
import { realpathSync } from 'node:fs'
import { delimiter, join, resolve } from 'node:path'

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

const web = resolve(import.meta.dir, '../packages/web')
const vite = Bun.spawn([node, join(web, 'node_modules/vite/bin/vite.js'), ...Bun.argv.slice(2)], {
  cwd: web,
  stdio: ['inherit', 'inherit', 'inherit'],
})
// An interrupt or a stop reaches Vite, and this ends when it does.
for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => vite.kill(signal))
}
process.exit(await vite.exited)
