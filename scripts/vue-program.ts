/**
 * A TypeScript program of a Vue project, templates included, built with the
 * Volar and Vue language APIs vue-tsc itself is built on. vue-tsc teaches tsc
 * about .vue files by rewriting tsc.js as Node reads it through
 * fs.readFileSync; Bun's require does not read through fs, so on the Bun
 * runtime this repository runs its toolchain on (bunfig.toml), vue-tsc runs
 * plain tsc and skips every .vue file. The type check and the UI text check
 * build their programs here instead.
 */
import { dirname } from 'node:path'
import ts from 'typescript'
import { proxyCreateProgram } from '@volar/typescript'
import { createParsedCommandLine, createVueLanguagePlugin, type Language } from '@vue/language-core'

export type VueProgram =
  | { ok: true; program: ts.Program; language: Language<string>; errors: readonly ts.Diagnostic[] }
  | { ok: false; errors: readonly ts.Diagnostic[] }

/**
 * The program of the project `project` (a tsconfig path) over its own files,
 * or over `rootNames` when given, with the language that maps each .vue
 * file's generated code back to the file.
 */
export function createVueProgram(project: string, rootNames?: readonly string[]): VueProgram {
  const config = ts.readConfigFile(project, ts.sys.readFile)
  if (config.error)
    return { ok: false, errors: [config.error] }
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, dirname(project), undefined, project, undefined, [
    { extension: 'vue', isMixedContent: true, scriptKind: ts.ScriptKind.Deferred },
  ])
  const { vueOptions } = createParsedCommandLine(ts, ts.sys, project)
  // The language plugin turns a .vue file into TypeScript; TypeScript only has to accept the extension.
  const options = { ...parsed.options, allowNonTsExtensions: true }
  let language: Language<string> | undefined
  const createProgram = proxyCreateProgram(ts, ts.createProgram, (ts, program) => ({
    languagePlugins: [createVueLanguagePlugin<string>(ts, program.options, vueOptions, (id) => id)],
    setup(created) {
      language = created
    },
  }))
  const program = createProgram({
    rootNames: rootNames ?? parsed.fileNames,
    options,
    host: ts.createCompilerHost(options),
    projectReferences: parsed.projectReferences,
  })
  if (!language)
    throw new Error('the Vue language plugin was not set up')
  return { ok: true, program, language, errors: parsed.errors }
}
