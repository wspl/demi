import {
  applyTranscriptPatches,
  type ClientSessionEvent,
} from '@demicodes/web-ui/transport/protocol'
import type { Conversation } from '../state/types'
import { transcriptTerminals } from './terminals'

/** Map agent events to the product's child and terminal presentation records. */
export function updateLiveStatus(conversation: Conversation): void {
  if (conversation.phase !== 'idle') {
    conversation.status = 'active'
    return
  }
  const last = conversation.blocks.at(-1)
  conversation.status =
    conversation.lastError || last?.type === 'error'
      ? 'error'
      : last?.type === 'abort'
        ? 'aborted'
        : conversation.blocks.length
          ? 'done'
          : 'idle'
}

export function applyConversationEvent(
  conversation: Conversation,
  event: ClientSessionEvent,
): void {
  updateLiveStatus(conversation)
  if (event.type === 'transcript_reset' || event.type === 'transcript_patch') {
    const stored = transcriptTerminals(conversation.blocks)
    for (const terminal of stored) {
      const current = conversation.terminals.find((item) => item.id === terminal.id)
      if (!current) {
        conversation.terminals.push(terminal)
      } else if (terminal.phase === 'exited') {
        Object.assign(current, terminal)
      } else {
        current.name = terminal.name
      }
    }
  }
  if (event.type === 'subagent') {
    const job = event.job
    const existing = conversation.subagents.find(
      (agent) => agent.id === job.subagentId,
    )
    if (existing) {
      existing.phase = job.phase
      if (event.event === 'closed') {
        existing.endedAt = job.endedAt ?? undefined
      }
    } else {
      conversation.subagents.push({
        id: job.subagentId,
        name: job.description,
        phase: job.phase,
        startedAt: job.startedAt,
        endedAt: job.endedAt ?? undefined,
        blocks: [],
      })
    }
  } else if (
    event.type === 'subagent_transcript_reset' ||
    event.type === 'subagent_transcript_patch'
  ) {
    const child = conversation.subagents.find(
      (agent) => agent.id === event.subagentId,
    )
    if (child) {
      child.blocks =
        event.type === 'subagent_transcript_reset'
          ? event.blocks
          : applyTranscriptPatches(child.blocks, event.patches)
    }
  } else if (event.type === 'shell_output') {
    const current = conversation.terminals.find(
      (terminal) => terminal.id === event.commandId,
    )
    const status = event.status
    const snapshot = {
      id: event.commandId,
      name: event.shellId,
      phase:
        status.status === 'running' ? ('running' as const) : ('exited' as const),
      startedAt:
        current?.startedAt ?? new Date(Date.now() - status.runningMs).toISOString(),
      ...(status.status !== 'running' ? { endedAt: new Date().toISOString() } : {}),
      output: status.output.tail || status.output.text,
    }
    if (current) {
      Object.assign(current, snapshot)
    } else {
      conversation.terminals.push(snapshot)
    }
  }
}
