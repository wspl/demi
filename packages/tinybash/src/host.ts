// What tinybash asks of its embedder: a filesystem, the stdio of a script,
// and a way to run root commands. tinybash owns these declarations the way
// any standalone shell owns its system interface; an embedder adapts its
// own filesystem and command loader to them (`@demicodes/host-virtual`
// does so for Demi's Host contract and loader).

export interface TinybashStat {
  isFile: boolean
  isDirectory: boolean
  isSymbolicLink: boolean
  mode: number
  size: number
  mtime: Date
  nlink?: number
  isCharacterDevice?: boolean
  isFIFO?: boolean
}

export interface TinybashDirent {
  name: string
  isFile: boolean
  isDirectory: boolean
  isSymbolicLink: boolean
}

/** The filesystem the builtins, redirections and globs run over. Paths are absolute or relative to `cwd`. */
export interface TinybashFs {
  readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array>
  writeFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  appendFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  exists(path: string, options?: { cwd?: string }): Promise<boolean>
  stat(path: string, options?: { cwd?: string }): Promise<TinybashStat>
  lstat(path: string, options?: { cwd?: string }): Promise<TinybashStat>
  readdir(path: string, options?: { cwd?: string; withFileTypes?: false }): Promise<string[]>
  readdir(path: string, options: { cwd?: string; withFileTypes: true }): Promise<TinybashDirent[]>
  mkdir(path: string, options?: { cwd?: string; recursive?: boolean }): Promise<void>
  rm(path: string, options?: { cwd?: string; recursive?: boolean; force?: boolean }): Promise<void>
  cp(path: string, destination: string, options?: { cwd?: string; recursive?: boolean }): Promise<void>
  mv(path: string, destination: string, options?: { cwd?: string }): Promise<void>
  readlink(path: string, options?: { cwd?: string }): Promise<string>
  chmod(path: string, mode: number, options?: { cwd?: string }): Promise<void>
  utimes(path: string, atime: Date, mtime: Date, options?: { cwd?: string }): Promise<void>
}

export type TinybashWriter = (data: string | Uint8Array) => Promise<void> | void

/** A script's stdout and stderr. */
export interface TinybashIO {
  stdout: TinybashWriter
  stderr: TinybashWriter
}

/** The stdio and environment of one root-command invocation, as the shell hands it to the dispatcher. */
export interface DispatchIO {
  /** The finite pipe: a pipeline, heredoc or `<` file. Absent for the script's live stdin. */
  stdin?: AsyncIterable<Uint8Array>
  /**
   * The script's own stdin, when this command's stdin is not redirected:
   * what the shell's caller writes after the command started. Live — it
   * ends when the caller ends it, never on its own.
   */
  stdinStream?: AsyncIterable<Uint8Array>
  stdout: TinybashWriter
  stderr: TinybashWriter
  cwd: string
  env: Record<string, string>
  signal?: AbortSignal
}

/**
 * What the shell asks of a root command before running a script: the path
 * arguments of one invocation (argv without the root name), so every path
 * can be checked against the namespace (`tinybash.md` § Interface).
 */
export type RootPaths = (argv: readonly string[]) => readonly string[]
