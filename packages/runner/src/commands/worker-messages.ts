import type { CommandContext } from '@demicodes/shell'

export type RunCommand = Pick<CommandContext, 'args' | 'cwd' | 'env'> & {
  type: 'run'
  path: string
}

export type InputRequest = { type: 'input'; id: number }
export type OutputRequest = {
  type: 'output'
  id: number
  stream: 'stdout' | 'stderr'
  bytes: Uint8Array
}
export type IORequest = InputRequest | OutputRequest
export type IOReplyValue = Uint8Array | null

export type WorkerMessage =
  | IORequest
  | { type: 'exit'; exitCode: number }
  | { type: 'error'; message: string }

export type RunnerMessage =
  | RunCommand
  | { type: 'reply'; id: number; value: IOReplyValue }
  | { type: 'reply'; id: number; error: string }
