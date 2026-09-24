// Runs `bun test` with the arguments given, in a temporary directory of its
// own that goes once the tests end: whatever a test leaves there, such as a
// runner's state with its copies of native packages, a run leaves nothing.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const directory = mkdtempSync(join(tmpdir(), 'demi-tests-'))
const tests = Bun.spawn([process.execPath, 'test', ...Bun.argv.slice(2)], {
  env: { ...process.env, TMPDIR: directory, TEMP: directory, TMP: directory },
  stdio: ['inherit', 'inherit', 'inherit'],
})
// An interrupt reaches the tests too; the run still removes the directory
// once they have ended.
process.on('SIGINT', () => {})
process.on('SIGTERM', () => tests.kill('SIGTERM'))
const code = await tests.exited
rmSync(directory, { recursive: true, force: true })
process.exit(code)
