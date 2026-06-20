export interface Host {
  defaultCwd: string
  fs: HostFileSystem
  process: HostProcess
  store: HostStore
}

export interface HostFileSystem {
  readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array>
  writeFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  appendFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  exists(path: string, options?: { cwd?: string }): Promise<boolean>
  stat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  lstat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  readdir(path: string, options?: { cwd?: string; withFileTypes?: false }): Promise<string[]>
  readdir(path: string, options: { cwd?: string; withFileTypes: true }): Promise<HostDirent[]>
  mkdir(path: string, options?: { cwd?: string; recursive?: boolean }): Promise<void>
  rm(path: string, options?: { cwd?: string; recursive?: boolean; force?: boolean }): Promise<void>
  cp(path: string, destination: string, options?: { cwd?: string; recursive?: boolean }): Promise<void>
  mv(path: string, destination: string, options?: { cwd?: string }): Promise<void>
  chmod(path: string, mode: number, options?: { cwd?: string }): Promise<void>
  symlink(target: string, path: string, options?: { cwd?: string }): Promise<void>
  link(existingPath: string, path: string, options?: { cwd?: string }): Promise<void>
  readlink(path: string, options?: { cwd?: string }): Promise<string>
  realpath(path: string, options?: { cwd?: string }): Promise<string>
  utimes(path: string, atime: Date, mtime: Date, options?: { cwd?: string }): Promise<void>
}

export interface HostProcess {
  spawn(params: HostSpawnParams): Promise<HostSpawnHandle>
}

export interface HostStore {
  readJson<T>(key: string): Promise<T | null>
  writeJson<T>(key: string, value: T): Promise<void>
  delete(key: string): Promise<void>
  list(prefix: string): Promise<string[]>
}

export interface HostFileStat {
  isFile: boolean
  isDirectory: boolean
  isSymbolicLink: boolean
  mode: number
  size: number
  mtime: Date
}

export interface HostDirent {
  name: string
  isFile: boolean
  isDirectory: boolean
  isSymbolicLink: boolean
}

export interface HostSpawnParams {
  command: string
  args?: string[]
  cwd?: string
  env?: Record<string, string | undefined>
  killProcessGroup?: boolean
}

export interface HostSpawnHandle {
  stdout: AsyncIterable<Uint8Array>
  stderr: AsyncIterable<Uint8Array>
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(signal?: string): Promise<void>
  wait(): Promise<HostSpawnExit>
}

export interface HostSpawnExit {
  exitCode: number | null
  signal?: string
}
