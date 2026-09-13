// View mapping between session-side values and server frames: the untyped
// tool-progress channel rendered into typed frame payloads (tools are
// arbitrary, so their progress is a real validation boundary).
import { safeJsonStringify } from '@demicodes/utils'
import { z } from 'zod'
import type {
  Block,
  ProviderErrorDiagnostics,
  ToolResultContentBlock
} from '@demicodes/core'
import type { ShellCommandStatusLike } from '../protocol/frames'
import { shellCommandStatusSchema } from '../protocol/schemas'
import { ProviderStreamError } from '../session/provider-stream-error'
import type { ShellToolView } from '../tools'

export function progressToOutput(progress: unknown): ToolResultContentBlock[] {
  return [{ type: 'text', text: progressToText(progress) }]
}

function progressToText(progress: unknown): string {
  if (typeof progress === 'string')
    return progress
  if (typeof progress === 'bigint')
    return progress.toString()
  if (typeof progress === 'symbol')
    return String(progress)
  if (typeof progress === 'function')
    return `[Function ${progress.name || 'anonymous'}]`
  return safeJsonStringify(progress) ?? String(progress)
}

/**
 * The `view` a shell tool call stores on its block. `shellToolView` in
 * `../tools` produces it; a transcript is storage, so every reader — this
 * replay, the terminal list, the change list — validates it against this one
 * declaration, and a view from another tool is simply not one.
 */
export const shellToolViewSchema: z.ZodType<ShellToolView> = z.object({
  kind: z.literal('shell'),
  status: z.enum(['running', 'exited', 'aborted']),
  shellId: z.string(),
  commandId: z.string(),
  exitCode: z.number().optional(),
  runningMs: z.number(),
  idleMs: z.number(),
  chunks: z.array(z.object({
    stream: z.enum(['stdout', 'stderr']),
    text: z.string(),
  })),
  viewTruncated: z.boolean(),
  files: z.array(z.object({
    path: z.string().min(1),
    kind: z.enum(['added', 'modified', 'deleted', 'renamed']),
    from: z.string().optional(),
    added: z.number(),
    removed: z.number(),
  })).optional(),
})

/**
 * The commands a transcript last saw running. The stored view is history:
 * whether such a command is still alive is the environment's to say.
 */
export function storedRunningCommandIds(blocks: readonly Block[]): string[] {
  const ids = new Set<string>()
  for (const block of blocks) {
    if (block.type !== 'tool_call')
      continue
    const view = shellToolViewSchema.safeParse(block.view)
    if (!view.success)
      continue
    if (view.data.status === 'running')
      ids.add(view.data.commandId)
    else
      ids.delete(view.data.commandId)
  }
  return [...ids]
}

/**
 * A shell command's status, when that is what a tool reported as progress.
 * Tools are arbitrary, so the channel is `unknown`; the frame's own schema
 * says what a shell status is, and progress that is not one is not a
 * `shell_output` frame.
 */
export function progressToShellOutput(
  progress: unknown,
): {
  shellId: string;
  commandId: string;
  status: ShellCommandStatusLike
} | null {
  const parsed = shellCommandStatusSchema.safeParse(progress)
  if (!parsed.success)
    return null
  return {
    shellId: parsed.data.shellId,
    commandId: parsed.data.commandId,
    status: parsed.data,
  }
}

export function errorDiagnostics(
  error: unknown
): ProviderErrorDiagnostics | undefined {
  if (!(error instanceof ProviderStreamError))
    return undefined
  return error.diagnostics
}
