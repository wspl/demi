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
import { ProviderStreamError } from '../session/provider-stream-error'

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

// Schemas validate; the original object is what crosses (no lossy clone).
const shellStreamViewSchema = z.looseObject({
  path: z.string().optional(),
  offset: z.number(),
  delta: z.string(),
  tail: z.string(),
  bytes: z.number(),
  truncated: z.boolean(),
})

const shellCommandStatusSchema = z.looseObject({
  shellId: z.string(),
  commandId: z.string(),
  status: z.enum(['running', 'exited', 'aborted']),
  stdout: shellStreamViewSchema,
  stderr: shellStreamViewSchema,
  runningMs: z.number(),
  idleMs: z.number(),
})

// The stored view of a shell tool call (`ShellToolView`): only the fields the
// replay needs, so a view from another tool is simply not one.
const storedShellViewSchema = z.looseObject({
  kind: z.literal('shell'),
  status: z.enum(['running', 'exited', 'aborted']),
  commandId: z.string(),
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
    const view = storedShellViewSchema.safeParse(block.view)
    if (!view.success)
      continue
    if (view.data.status === 'running')
      ids.add(view.data.commandId)
    else
      ids.delete(view.data.commandId)
  }
  return [...ids]
}

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
    status: progress as ShellCommandStatusLike,
  }
}

export function errorDiagnostics(
  error: unknown
): ProviderErrorDiagnostics | undefined {
  if (!(error instanceof ProviderStreamError))
    return undefined
  return error.diagnostics
}
