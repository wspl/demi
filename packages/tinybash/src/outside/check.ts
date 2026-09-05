import type { RootAdmission, RootPaths, TinybashFs } from '../host'
import type { Command, Script } from '../grammar/ast'
import { type ExpansionScope, type Piece, expandSingle, expandToFields, fieldText } from '../grammar/expand'
import { hasGlobChars } from '../grammar/glob'
import { BUILTINS } from '../builtins/table'
import { basenamePath, normalizePath } from '@demicodes/utils'
import { insideNamespace, resolvePath } from './namespace'
import { outside } from './reasons'

export interface CheckContext {
  admitRoot?: RootAdmission
  roots: ReadonlyMap<string, RootPaths>
  namespace: readonly string[]
  scope: ExpansionScope
  /** When present, a `cd` before any command that can create or remove directories is decided against it. */
  fs?: TinybashFs
}

/** The shell state a script may be in at some point: the cwd and the variables. */
interface Snapshot {
  cwd: string
  vars: Record<string, string>
}

/** Builtins after which a directory may exist that did not, or vice versa. */
const RESHAPING_BUILTINS = new Set(['mkdir', 'rm', 'mv', 'cp'])

/** Beyond this many candidate states the script is handed to a machine rather than analysed further. */
const MAX_STATES = 1024

/**
 * The parse-first decision: every command word a builtin or a root, every flag
 * on its whitelist, every path inside the namespace under every shell state
 * the script can reach. Every string in this subset is static — no command
 * output can reach a variable — so the only run-time unknown is whether a
 * `cd` succeeds. The check therefore carries a set of candidate states: a
 * `cd` whose outcome cannot be decided adds the state where it succeeded
 * beside the one where it failed, and a path is accepted only if it stays
 * inside under all of them. A glob never changes a path's depth (each
 * pattern segment matches exactly one name, never `.` or `..`, and the
 * pattern is kept literally when nothing matches), so the unexpanded text
 * is checked as written. Throws `OutsideError` at the first condition.
 */
export async function checkScript(script: Script, context: CheckContext): Promise<void> {
  const home = context.scope.home
  let states: Snapshot[] = [{ cwd: context.scope.cwd, vars: { ...context.scope.vars } }]
  let reshaped = false
  const scopeOf = (state: Snapshot): ExpansionScope => ({ home, cwd: state.cwd, vars: state.vars })
  const checkPath = (cwd: string, path: string, line: number) => {
    if (!insideNamespace(resolvePath(cwd, path), context.namespace)) outside({ kind: 'path', path, line })
  }
  const checkField = (cwd: string, field: readonly Piece[], line: number) => {
    const text = fieldText(field)
    if (hasGlobChars(field) && text.includes('**')) outside({ kind: 'grammar', found: '**', why: 'recursive globbing is not expanded here', wayOut: 'find, or list the names', line })
    checkPath(cwd, text, line)
  }

  const checkCommand = async (command: Command, inPipeline: boolean, conditional: boolean) => {
    for (const state of states) {
      const scope = scopeOf(state)
      for (const redirect of command.redirects) {
        if (redirect.kind !== 'file' && redirect.kind !== 'input') continue
        for (const field of expandToFields(redirect.path, scope)) checkField(state.cwd, field, redirect.line)
      }
    }
    if (command.words.length === 0) {
      if (!inPipeline) {
        // A conditional assignment may not run: keep the state where it did not.
        const assigned = states.map((state) => ({ cwd: state.cwd, vars: { ...state.vars } }))
        for (const state of assigned) {
          const scope = scopeOf(state)
          for (const assignment of command.assignments) state.vars[assignment.name] = expandSingle(assignment.value, scope)
        }
        states = dedupe(conditional ? [...states, ...assigned] : assigned)
      }
      return
    }
    const next: Snapshot[] = []
    for (const state of states) {
      const scope = scopeOf(state)
      const fields: { field: Piece[]; line: number }[] = []
      for (const word of command.words) for (const field of expandToFields(word, scope)) fields.push({ field, line: word.line })
      if (fields.length === 0) {
        next.push(state)
        continue
      }
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
        if (context.admitRoot && (fields.some(entry => hasGlobChars(entry.field)) || !context.admitRoot(name, argv.slice(1)))) {
          outside({ kind: 'admission', name, line })
        }
        paths = root(argv.slice(1))
        reshaped = true
      }
      // Map each path back to the field it came from for its line; a path a builtin derives itself is checked as text.
      const used = new Set<number>()
      for (const path of paths) {
        const index = argv.findIndex((arg, i) => i > 0 && arg === path && !used.has(i))
        if (index === -1) {
          checkPath(state.cwd, path, line)
          continue
        }
        used.add(index)
        checkField(state.cwd, fields[index]!.field, fields[index]!.line)
      }
      // `mv`/`cp` into a directory land at `dest/<last component of source>`, which `..` can carry
      // outside. A glob in the last operand may expand to several names, of which all but the last
      // become sources, so every operand is checked as a source of the last one.
      if ((name === 'mv' || name === 'cp') && paths.length >= 2) {
        const destination = paths[paths.length - 1]!
        for (const source of paths) checkPath(state.cwd, `${destination}/${basenamePath(source)}`, line)
      }
      if (RESHAPING_BUILTINS.has(name)) reshaped = true
      if (name === 'cd' && !inPipeline) {
        next.push(...(await afterCd(state, argv, fields[1]?.field, conditional)))
      } else {
        next.push(state)
      }
    }
    states = dedupe(next)
    if (states.length > MAX_STATES) outside({ kind: 'grammar', found: 'cd', why: 'too many directory changes to follow', wayOut: 'a machine', line: command.line })
  }

  /**
   * The states after a top-level `cd`: the one where it succeeded, the one
   * where it failed, or both. A `cd` is decided when it runs unconditionally
   * and its outcome cannot depend on the script so far: the home and the
   * namespace roots exist by contract, and any other target is looked up
   * when nothing before it could have created or removed a directory.
   */
  const afterCd = async (state: Snapshot, argv: readonly string[], target: readonly Piece[] | undefined, conditional: boolean): Promise<Snapshot[]> => {
    if (argv.length > 2) return [state]
    const resolved = argv.length === 2 ? resolvePath(state.cwd, argv[1]!) : home
    const moved: Snapshot = { cwd: resolved, vars: state.vars }
    if (conditional || (target && hasGlobChars(target))) return [state, moved]
    if (resolved === home || context.namespace.some((prefix) => normalizePath(prefix) === resolved)) return [moved]
    if (context.fs && !reshaped) {
      try {
        return (await context.fs.stat(resolved)).isDirectory ? [moved] : [state]
      } catch {
        return [state]
      }
    }
    return [state, moved]
  }

  for (const statement of script.statements) {
    const pipelines = [statement.first, ...statement.rest.map((entry) => entry.pipeline)]
    for (const [index, pipeline] of pipelines.entries()) {
      for (const command of pipeline.commands) await checkCommand(command, pipeline.commands.length > 1, index > 0)
    }
  }
}

/** The last path component as `mv`/`cp` name the target (`a/b/` → `b`, `a/..` → `..`). */

function dedupe(states: Snapshot[]): Snapshot[] {
  const seen = new Map<string, Snapshot>()
  for (const state of states) {
    const key = `${state.cwd}\0${JSON.stringify(state.vars)}`
    if (!seen.has(key)) seen.set(key, { cwd: state.cwd, vars: { ...state.vars } })
  }
  return [...seen.values()]
}
