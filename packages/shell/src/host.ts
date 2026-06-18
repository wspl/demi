export interface Host {
  root: string
  spawn(params: HostSpawnParams): Promise<HostSpawnHandle>
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
  kill(signal?: NodeJS.Signals): Promise<void>
  wait(): Promise<HostSpawnExit>
}

export interface HostSpawnExit {
  exitCode: number | null
  signal?: string
}
