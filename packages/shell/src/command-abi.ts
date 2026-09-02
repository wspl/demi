import type { z } from 'zod'
import type { HostFileSystem } from './host'

/**
 * The command ABI (`docs/demi-next/commands.md` § The command ABI): what a
 * `runtime` module sees, and how a tree carries one.
 */

/** A writer for stdout or stderr: text is UTF-8, bytes pass through. */
export type CommandWriter = (data: string | Uint8Array) => Promise<void> | void

/** The whole world a `runtime` module sees. */
export interface CommandContext<Args = Record<string, unknown>> {
  /** The parsed arguments, already validated against the leaf's input schema. */
  args: Args
  /** The filesystem of the Host the command runs against. */
  fs: HostFileSystem
  /** The invoking shell's working directory. */
  cwd: string
  /** The invoking shell's exported environment. */
  env: Record<string, string>
  /** The command's stdin as a byte stream. */
  stdin: AsyncIterable<Uint8Array>
  stdout: CommandWriter
  stderr: CommandWriter
  signal: AbortSignal
}

export interface CommandResult {
  exitCode: number
}

/** The one export of a `runtime` module. */
export type CommandModule<Args = Record<string, unknown>> = (ctx: CommandContext<Args>) => Promise<CommandResult>

/**
 * The stdio and environment of one command invocation, as a shell hands it
 * to a dispatcher (the loader): tinybash and the just-bash bridge both
 * speak this.
 */
export interface DispatchIO {
  /** The pipe: a pipeline, heredoc, `<` file, or empty. Finite. */
  stdin: AsyncIterable<Uint8Array>
  /**
   * The script's own stdin, when this command's stdin is not redirected:
   * what the shell's caller writes after the command started (`shell_write`).
   * Live — it ends when the caller ends it, never on its own.
   */
  stdinStream?: AsyncIterable<Uint8Array>
  stdout: CommandWriter
  stderr: CommandWriter
  cwd: string
  env: Record<string, string>
  signal?: AbortSignal
}

/**
 * What a hostless shell asks of a root command before running a script: the
 * path arguments of one invocation (argv without the root name), from the
 * path marks on the tree (`tinybash.md` § Interface).
 */
export type RootPaths = (argv: readonly string[]) => readonly string[]

declare const runtimeModuleBrand: unique symbol

/** The text of a `runtime` module, as a tree carries it. */
export type RuntimeModule = string & { readonly [runtimeModuleBrand]: true }

/**
 * The one conversion from a text import (a `*.command.ts` file imported
 * with the `text` attribute) to a `RuntimeModule`. TypeScript types that
 * import as the module rather than as a string, and a build that lost the
 * text (the `commandModulesAsText` plugin missing) delivers a function
 * here; both are caught at this single point.
 */
export function runtimeModule(source: unknown): RuntimeModule {
  if (typeof source !== 'string') {
    throw new TypeError(`runtimeModule: expected the module text, received ${typeof source}; import the module with { type: 'text' } and build with commandModulesAsText`)
  }
  return source as RuntimeModule
}

const PATH_MARK = 'path'

/** Marks an argument as naming a file or directory (`commands.md` § The command ABI). */
export function pathArg<T extends z.ZodType>(schema: T): T {
  return schema.meta({ ...schema.meta(), [PATH_MARK]: true }) as T
}

export function isPathArg(schema: z.ZodType): boolean {
  return schema.meta()?.[PATH_MARK] === true
}

const loadedModules = new Map<string, Promise<CommandModule>>()

/** Loads a `runtime` module from its JavaScript text, once per text. */
export function loadCommandModule(javascript: string): Promise<CommandModule> {
  let loaded = loadedModules.get(javascript)
  if (!loaded) {
    loaded = importModule(javascript)
    loadedModules.set(javascript, loaded)
    loaded.catch(() => loadedModules.delete(javascript))
  }
  return loaded
}

async function importModule(javascript: string): Promise<CommandModule> {
  const url = URL.createObjectURL(new Blob([javascript], { type: 'text/javascript' }))
  try {
    return await importCommandModule(url)
  } finally {
    URL.revokeObjectURL(url)
  }
}

/**
 * Imports a `runtime` module by specifier — a `blob:` URL here, a file in the
 * manifest cache on a target — and checks its one export.
 */
export async function importCommandModule(specifier: string): Promise<CommandModule> {
  const loaded = (await import(specifier)) as { default?: unknown }
  if (typeof loaded.default !== 'function') {
    throw new TypeError('a runtime module must default-export its command function')
  }
  return loaded.default as CommandModule
}
