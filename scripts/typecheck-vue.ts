/**
 * Type-checks Vue projects, templates included:
 * `bun scripts/typecheck-vue.ts <tsconfig>...` prints every error and exits 2
 * when there is one. The program is the one vue-tsc would build
 * (`vue-program.ts` says why this does not run vue-tsc).
 */
import { resolve } from 'node:path'
import ts from 'typescript'
import { createVueProgram } from './vue-program'

function check(project: string): readonly ts.Diagnostic[] {
  const created = createVueProgram(project)
  if (!created.ok)
    return created.errors
  return [...created.errors, ...ts.getPreEmitDiagnostics(created.program)]
}

const projects = process.argv.slice(2)
if (projects.length === 0) {
  console.error('Usage: bun scripts/typecheck-vue.ts <tsconfig>...')
  process.exit(1)
}
const host: ts.FormatDiagnosticsHost = {
  getCanonicalFileName: (fileName) => fileName,
  getCurrentDirectory: () => process.cwd(),
  getNewLine: () => ts.sys.newLine,
}
const format = process.stdout.isTTY ? ts.formatDiagnosticsWithColorAndContext : ts.formatDiagnostics
let errors = 0
for (const project of projects) {
  const diagnostics = check(resolve(project))
  errors += diagnostics.length
  if (diagnostics.length > 0)
    process.stdout.write(format(diagnostics, host))
}
if (errors > 0) {
  console.log(`Found ${errors} error${errors === 1 ? '' : 's'}.`)
  process.exit(2)
}
