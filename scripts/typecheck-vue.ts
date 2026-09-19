/**
 * Type-checks Vue projects, templates included:
 * `bun scripts/typecheck-vue.ts <tsconfig>...` prints every error and exits 2
 * when there is one.
 *
 * vue-tsc teaches tsc about .vue files by rewriting tsc.js as Node reads it
 * through fs.readFileSync. Bun's require does not read through fs, so on the
 * Bun runtime this repository runs its toolchain on (bunfig.toml), vue-tsc
 * runs plain tsc, skips every .vue file and exits 0. This builds the program
 * with the Volar and Vue language APIs vue-tsc itself is built on, and reports
 * the same diagnostics without the rewrite.
 */
import { dirname, resolve } from 'node:path'
import ts from 'typescript'
import { proxyCreateProgram } from '@volar/typescript'
import { createParsedCommandLine, createVueLanguagePlugin } from '@vue/language-core'

function check(project: string): readonly ts.Diagnostic[] {
  const config = ts.readConfigFile(project, ts.sys.readFile)
  if (config.error)
    return [config.error]
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, dirname(project), undefined, project, undefined, [
    { extension: 'vue', isMixedContent: true, scriptKind: ts.ScriptKind.Deferred },
  ])
  const { vueOptions } = createParsedCommandLine(ts, ts.sys, project)
  // The language plugin turns a .vue file into TypeScript; TypeScript only has to accept the extension.
  const options = { ...parsed.options, allowNonTsExtensions: true }
  const createProgram = proxyCreateProgram(ts, ts.createProgram, (ts, program) => [
    createVueLanguagePlugin<string>(ts, program.options, vueOptions, (id) => id),
  ])
  const program = createProgram({
    rootNames: parsed.fileNames,
    options,
    host: ts.createCompilerHost(options),
    projectReferences: parsed.projectReferences,
  })
  return [...parsed.errors, ...ts.getPreEmitDiagnostics(program)]
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
