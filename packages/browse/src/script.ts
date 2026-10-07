// A call's script (browse.md § A call): the agent's JavaScript, run as the
// body of an async function in which the scope's names are defined. The
// script becomes a module of its own in the slot's folder, its first line
// on the module's first line after the function's head, so the module's
// line numbers are the script's: a failure's stack, or a syntax error's
// position, names the script's line without any arithmetic. Bun transpiles
// the module as TypeScript, so a script may also annotate types.
import { mkdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

/** The names a script can use, in the order the function's head lists them. */
export const SCOPE_NAMES = ['page', 'context', 'expect', 'cdp', 'demi', 'keep', 'console'] as const
export type ScriptScope = Record<typeof SCOPE_NAMES[number], unknown>

/** The module's head, which the script's first line follows on the same line. */
const HEAD = `export default async ({ ${SCOPE_NAMES.join(', ')} }) => { `

/** The calls this process has run, which name their modules: a module, once imported, stays in Bun's registry. */
let written = 0

export class Script {
  /** The module the script runs as. */
  readonly file: string

  /** Writes `source` as a module in `folder`. */
  constructor(readonly source: string, folder: string) {
    mkdirSync(folder, { recursive: true })
    written += 1
    // Bun names the module by its real path in a syntax error's position, as
    // in a stack: the folder's own, beneath any link such as macOS's /var.
    this.file = join(realpathSync(folder), `call-${process.pid}-${written}.ts`)
    // The closing brace goes on a line of its own, after a script whose last line is a comment.
    writeFileSync(this.file, `${HEAD}${source}\n}\n`)
  }

  /** Runs the script in `scope`; answers what it returned. */
  async run(scope: ScriptScope): Promise<unknown> {
    const module: unknown = await import(this.file)
    const main = typeof module === 'object' && module !== null && 'default' in module ? module.default : undefined
    if (typeof main !== 'function') {
      throw new Error(`${this.file} has no default export: the script closed the function it runs in`)
    }
    return main(scope)
  }

  /**
   * The script's line `error` was thrown at, or null when no line of the
   * script is in its stack, as for an error an event threw: the innermost
   * frame in the script, such as the line of a callback the script gave a
   * helper, or a syntax error's position.
   */
  lineOf(error: unknown): number | null {
    const compiled = buildMessageOf(error)
    if (compiled) {
      return compiled.position?.file === this.file ? compiled.position.line : null
    }
    if (!(error instanceof Error) || error.stack === undefined) {
      return null
    }
    const frame = new RegExp(`${RegExp.escape(this.file)}:(\\d+):\\d+`).exec(error.stack)
    return frame ? Number(frame[1]) : null
  }

  /** The text of the script's line `line`. */
  text(line: number): string {
    return this.source.split('\n')[line - 1]?.trim() ?? ''
  }

  remove(): void {
    rmSync(this.file, { force: true })
  }
}

/**
 * The message Bun threw when the script's module does not compile, such as
 * `Unexpected }` at its line; several come as an AggregateError, whose first
 * is the one to fix first.
 */
export function buildMessageOf(error: unknown): BuildMessage | null {
  if (error instanceof BuildMessage) {
    return error
  }
  if (error instanceof AggregateError) {
    const first: unknown = error.errors[0]
    return first instanceof BuildMessage ? first : null
  }
  return null
}
