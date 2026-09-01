// View mapping between session-side values and server frames: conversation
// summaries for `list_conversations`, and the untyped tool-progress channel
// rendered into typed frame payloads (tools are arbitrary, so their progress
// is a real validation boundary).
import { isRecord, safeJsonStringify } from '@demicodes/utils'
import { z } from 'zod'
import type { Block, ProviderErrorDiagnostics, ToolResultContentBlock } from '@demicodes/core'
import type { BashAuditEvent } from '@demicodes/shell'
import type { ConversationSummary, ShellCommandStatusLike } from '../protocol/frames'
import type { AgentSessionCheckpoint } from '../types'
import { ProviderStreamError } from '../session/provider-stream-error'

export function summarizeConversation(id: string, checkpoint: AgentSessionCheckpoint<unknown>): ConversationSummary {
  const blocks = checkpoint.transcript.blocks
  const first = blocks[0]
  const last = blocks[blocks.length - 1]
  return {
    id,
    title: conversationTitle(blocks),
    createdAt: first?.createdAt ?? '',
    updatedAt: last?.createdAt ?? first?.createdAt ?? '',
  }
}

function conversationTitle(blocks: Block[]): string {
  const user = blocks.find((block): block is Extract<Block, { type: 'user' }> => block.type === 'user')
  const text = user?.content.find((item): item is { type: 'text'; text: string } => item.type === 'text')?.text
  const title = (text ?? '').replace(/\s+/g, ' ').trim()
  return title ? title.slice(0, 80) : 'Untitled conversation'
}

export function progressToOutput(progress: unknown): ToolResultContentBlock[] {
  return [{ type: 'text', text: progressToText(progress) }]
}

function progressToText(progress: unknown): string {
  if (typeof progress === 'string') return progress
  if (typeof progress === 'bigint') return progress.toString()
  if (typeof progress === 'symbol') return String(progress)
  if (typeof progress === 'function') return `[Function ${progress.name || 'anonymous'}]`
  return safeJsonStringify(progress) ?? String(progress)
}

// Schemas validate; the original object is what crosses (no lossy clone).
const shellStreamViewSchema = z.looseObject({
  path: z.string(),
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

const auditEventSchema = z.discriminatedUnion('kind', [
  z.looseObject({
    kind: z.literal('registered-command'),
    name: z.string(),
    args: z.array(z.string()),
    exitCode: z.number(),
  }),
  z.looseObject({
    kind: z.literal('portable-command'),
    name: z.string(),
    args: z.array(z.string()),
    cwd: z.string(),
    exitCode: z.number(),
  }),
  z.looseObject({
    kind: z.literal('system-command'),
    name: z.string(),
    args: z.array(z.string()),
    cwd: z.string(),
    exitCode: z.number().nullable(),
  }),
])

export function progressToShellOutput(
  progress: unknown,
): { shellId: string; commandId: string; status: ShellCommandStatusLike } | null {
  const parsed = shellCommandStatusSchema.safeParse(progress)
  if (!parsed.success) return null
  return {
    shellId: parsed.data.shellId,
    commandId: parsed.data.commandId,
    status: progress as ShellCommandStatusLike,
  }
}

export function progressToAudit(progress: unknown): BashAuditEvent[] {
  if (!isRecord(progress) || !Array.isArray(progress.audit)) return []
  return progress.audit.filter((event): event is BashAuditEvent => auditEventSchema.safeParse(event).success)
}

export function errorDiagnostics(error: unknown): ProviderErrorDiagnostics | undefined {
  if (!(error instanceof ProviderStreamError)) return undefined
  return error.diagnostics
}
