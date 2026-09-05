// The tinyjs API (`docs/demi-next/tinyjs.md` § The tinyjs API), declared
// once here: this package is the only importer of `tinyjs:*`.

declare module 'tinyjs:fs' {
  export type EntryKind = 'file' | 'dir' | 'symlink' | 'other'
  export interface Stat {
    kind: EntryKind
    mode: number
    size: number
    mtimeMs: number
    atimeMs: number
    uid: number
    gid: number
    ino: number
    dev: number
    nlink: number
  }
  export function readFile(path: string): Promise<Uint8Array>
  export function writeFile(path: string, data: Uint8Array, options?: { mode?: number; append?: boolean }): Promise<void>
  export function stat(path: string): Promise<Stat>
  export function lstat(path: string): Promise<Stat>
  export function readdir(path: string): Promise<Array<{ name: string; kind: EntryKind }>>
  export function mkdir(path: string, options?: { recursive?: boolean; mode?: number }): Promise<void>
  export function rmdir(path: string): Promise<void>
  export function unlink(path: string): Promise<void>
  export function rename(from: string, to: string): Promise<void>
  export function symlink(target: string, path: string): Promise<void>
  export function link(from: string, to: string): Promise<void>
  export function readlink(path: string): Promise<string>
  export function realpath(path: string): Promise<string>
  export function chmod(path: string, mode: number): Promise<void>
  export function utimes(path: string, atimeMs: number, mtimeMs: number): Promise<void>
  export function truncate(path: string, size: number): Promise<void>
  export function open(path: string, flags: string, mode?: number): Promise<number>
  export function read(fd: number, max: number, offset?: number): Promise<Uint8Array | null>
  export function write(fd: number, data: Uint8Array): Promise<void>
  export function close(fd: number): void
  /**
   * A bounded in-memory pipe. `write` blocks once the buffer is full (the
   * backpressure); closing `write` ends `read` with EOF; closing `read`
   * fails later writes with `EPIPE`.
   */
  export function pipe(): { read: number; write: number }
}

declare module 'tinyjs:process' {
  export interface SpawnOptions {
    command: string
    args?: string[]
    cwd?: string
    env?: Record<string, string>
    stdin?: 'pipe' | 'null'
    uid?: number
    gid?: number
    processGroup?: boolean
    /** With `stream`, the full stdout is also readable from `Child.stdoutStream`, with backpressure to the child. */
    tee?: { stdoutPath: string; stderrPath: string; viewLimit?: number; stream?: boolean }
  }
  export interface Child {
    pid: number
    stdin: number | null
    stdout: number
    stderr: number
    /** The full stdout as a stream when `tee.stream` was set; null otherwise. */
    stdoutStream: number | null
  }
  export interface WaitResult {
    code: number | null
    signal?: string
    stdoutBytes?: number
    stderrBytes?: number
  }
  export function spawn(options: SpawnOptions): Promise<Child>
  export function wait(pid: number): Promise<WaitResult>
  export function kill(pid: number, signal: string, options?: { group?: boolean }): void
}

declare module 'tinyjs:runtime' {
  export const argv: string[]
  export const env: Readonly<Record<string, string>>
  export function cwd(): string
  export function chdir(path: string): void
  export function exit(code?: number): never
  export function onSignal(name: 'SIGTERM' | 'SIGINT' | 'SIGHUP' | 'SIGUSR1' | 'SIGUSR2', handler: () => void): void
  export const stdin: number
  export const stdout: number
  export const stderr: number
  export const pid: number
  export const identity: { uid: number; gid: number; hostname: string; homeDir: string }
  export const version: number
  export const abi: number
  export function openHandles(): number
  /** `dev:ino` of the open file description behind an OS file descriptor, `null` when it is not open. */
  export function fdNode(fd: number): string | null
}

declare module 'tinyjs:bytes' {
  export function msgpackEncode(value: unknown): Uint8Array
  export function msgpackDecode(bytes: Uint8Array): unknown
  export function base64Encode(bytes: Uint8Array): string
  export function base64Decode(text: string): Uint8Array
  export function sha256(bytes: Uint8Array): Uint8Array
  export function randomBytes(n: number): Uint8Array
}

declare module 'tinyjs:net' {
  export function wsConnect(url: string, options?: { headers?: Record<string, string> }): Promise<number>
  export function wsSend(ws: number, data: Uint8Array): Promise<void>
  export function wsRecv(ws: number): Promise<Uint8Array | null>
  export function wsClose(ws: number, code?: number): void
  export function udsConnect(path: string): Promise<number>
  export function udsListen(path: string, options?: { mode?: number }): Promise<number>
  export function accept(listener: number): Promise<number>
  export function close(handle: number): void
  export function httpRequest(request: {
    method: string
    url: string
    headers?: Record<string, string>
    /** `{ handle }` streams a readable handle as the body and consumes it: the handle is closed by the request. */
    body?: Uint8Array | { file: string } | { handle: number }
  }): Promise<{ status: number; headers: Record<string, string>; body: number }>
}
