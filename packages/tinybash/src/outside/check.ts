import type { Command, Script, Word } from '../grammar/ast'
import { type ExpansionScope, type Piece, expandSingle, expandToFields, fieldText } from '../grammar/expand'
import { globPrefixDir, hasGlobChars } from '../grammar/glob'
import { BUILTINS } from '../builtins/table'
import { insideNamespace, resolvePath } from './namespace'
import { outside } from './reasons'

export type RootPaths = (argv: readonly string[]) => readonly string[]

export interface CheckContext {
  roots: ReadonlyMap<string, RootPaths>
  namespace: readonly string[]
  scope: ExpansionScope
}

/**
 * The parse-first decision: every command word a builtin or a root, every flag
 * on its whitelist, every path inside the namespace. Variables and `cd` are
 * simulated in order, which is exact because no command output can reach a
 * variable in this subset. Throws `OutsideError` at the first condition.
 */
export function checkScript(script: Script, context: CheckContext): void {
  const scope: ExpansionScope = { home: context.scope.home, cwd: context.scope.cwd, vars: { ...context.scope.vars } }
  const vars = scope.vars as Record<string, string>
  const checkPath = (path: string, line: number) => {
    if (!insideNamespace(resolvePath(scope.cwd, path), context.namespace)) outside({ kind: 'path', path, line })
  }
  const checkField = (field: readonly Piece[], line: number) => {
    const text = fieldText(field)
    if (hasGlobChars(field)) {
      if (text.includes('**')) outside({ kind: 'grammar', found: '**', why: 'recursive globbing is not expanded here', wayOut: 'find, or list the names', line })
      const prefix = globPrefixDir(field)
      if (prefix.startsWith('/')) checkPath(prefix, line)
      else checkPath(prefix === '' ? '.' : prefix, line)
      return
    }
    checkPath(text, line)
  }
  const checkCommand = (command: Command, inPipeline: boolean) => {
    for (const redirect of command.redirects) {
      if (redirect.kind === 'file' || redirect.kind === 'input') checkPath(expandSingle(redirect.path, scope), redirect.line)
    }
    if (command.words.length === 0) {
      if (!inPipeline) for (const assignment of command.assignments) vars[assignment.name] = expandSingle(assignment.value, scope)
      return
    }
    const fields: { field: Piece[]; line: number }[] = []
    for (const word of command.words) for (const field of expandToFields(word, scope)) fields.push({ field, line: word.line })
    if (fields.length === 0) return
    const argv = fields.map((entry) => fieldText(entry.field))
    const name = argv[0]!
    const line = command.line
    let paths: readonly string[]
    const builtin = BUILTINS.get(name)
    if (builtin) {
      paths = builtin.paths(argv.slice(1), line)
    } else {
      const root = context.roots.get(name)
      if (!root) outside({ kind: 'program', name, line })
      paths = root(argv.slice(1))
    }
    // Map each path back to the field it came from so a glob is checked by its prefix.
    const used = new Set<number>()
    for (const path of paths) {
      const index = argv.findIndex((arg, i) => i > 0 && arg === path && !used.has(i))
      if (index === -1) {
        checkPath(path, line)
        continue
      }
      used.add(index)
      checkField(fields[index]!.field, fields[index]!.line)
    }
    if (name === 'cd' && !inPipeline) scope.cwd = argv.length > 1 ? resolvePath(scope.cwd, argv[1]!) : scope.home
  }
  for (const statement of script.statements) {
    const pipelines = [statement.first, ...statement.rest.map((entry) => entry.pipeline)]
    for (const pipeline of pipelines) {
      for (const command of pipeline.commands) checkCommand(command, pipeline.commands.length > 1)
    }
  }
}

export function wordLine(word: Word): number {
  return word.line
}
