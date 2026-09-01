// View mapping between session-side values and server frames: conversation
// summaries for `list_conversations`, and the untyped tool-progress channel
// rendered into typed frame payloads (tools are arbitrary, so their progress
// is a real validation boundary).
import { isRecord, safeJsonStringify } from '@demicodes/utils'
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

export function progressToShellOutput(
  progress: unknown,
): { shellId: string; commandId: string; status: ShellCommandStatusLike } | null {
  if (!isRecord(progress)) return null
  if (typeof progress.shellId !== 'string' || typeof progress.commandId !== 'string') return null
  if (progress.status !== 'running' && progress.status !== 'exited' && progress.status !== 'aborted') return null
  if (!isRecord(progress.stdout) || !isRecord(progress.stderr)) return null
  const stdout = progress.stdout
  const stderr = progress.stderr
  if (
    !isShellStreamView(stdout) ||
    !isShellStreamView(stderr) ||
    typeof progress.runningMs !== 'number' ||
    typeof progress.idleMs !== 'number'
  ) {
    return null
  }
  return {
    shellId: progress.shellId,
    commandId: progress.commandId,
    status: progress as unknown as ShellCommandStatusLike,
  }
}

function isShellStreamView(value: Record<string, unknown>): boolean {
  return (
    typeof value.path === 'string' &&
    typeof value.offset === 'number' &&
    typeof value.delta === 'string' &&
    typeof value.tail === 'string' &&
    typeof value.bytes === 'number' &&
    typeof value.truncated === 'boolean'
  )
}

export function progressToAudit(progress: unknown): BashAuditEvent[] {
  if (!isRecord(progress) || !Array.isArray(progress.audit)) return []
  return progress.audit.filter(isBashAuditEvent)
}

function isBashAuditEvent(value: unknown): value is BashAuditEvent {
  if (!isRecord(value)) return false
  if (value.kind === 'registered-command') {
    return typeof value.name === 'string' && isStringArray(value.args) && typeof value.exitCode === 'number'
  }
  if (value.kind === 'portable-command') {
    return (
      typeof value.name === 'string' &&
      isStringArray(value.args) &&
      typeof value.cwd === 'string' &&
      typeof value.exitCode === 'number'
    )
  }
  if (value.kind === 'system-command') {
    return (
      typeof value.name === 'string' &&
      isStringArray(value.args) &&
      typeof value.cwd === 'string' &&
      (typeof value.exitCode === 'number' || value.exitCode === null)
    )
  }
  return false
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string')
}

export function errorDiagnostics(error: unknown): ProviderErrorDiagnostics | undefined {
  if (!(error instanceof ProviderStreamError)) return undefined
  return error.diagnostics
}
