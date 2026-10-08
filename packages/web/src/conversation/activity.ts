import {
  applyTranscriptPatches,
  type ClientSessionEvent,
} from '@demicodes/web-ui/transport/protocol'
import type { Conversation } from '../state/types'
import { callScript, transcriptTerminals } from './terminals'
import { isConversationActive } from '@demicodes/web-ui/agent/conversation-status'
import { followLiveOutput, type TerminalRecord } from '@demicodes/web-ui/agent/terminals'

/** Map agent events to the product's child and terminal presentation records. */
export function updateLiveStatus(conversation: Conversation): void {
  if (isConversationActive(conversation.phase, conversation.subagents)) {
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
  if (event.type === 'transcript_reset' || event.type === 'transcript_patch') {
    // A stored end (`endedAt`) is final, unless the page follows the command
    // live and keeps its live view; otherwise a stored view only names it.
    const stored = transcriptTerminals(conversation.blocks)
    for (const terminal of stored) {
      const current = conversation.terminals.find((item) => item.id === terminal.id)
      if (!current) {
        conversation.terminals.push(terminal)
      } else if (terminal.endedAt && current.chars === undefined) {
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
        pendingCalls: [],
        failures: {},
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
      if (event.type === 'subagent_transcript_reset') {
        child.blocks = event.blocks
        child.failures = event.failures
      } else {
        child.blocks = applyTranscriptPatches(child.blocks, event.patches)
        child.failures = { ...child.failures, ...event.failures }
      }
    }
  } else if (event.type === 'pending_calls' && event.subagentId !== undefined) {
    const child = conversation.subagents.find((agent) => agent.id === event.subagentId)
    if (child) {
      child.pendingCalls = event.pendingCalls
    }
  } else if (event.type === 'shell_output') {
    // A command's live view, the same for every page (`runtime.md` § Live
    // output): what it adds to the output shown, under its call while the
    // call runs and in the dock after.
    const { subagentId, status } = event
    const current = conversation.terminals.find(
      (terminal) => terminal.id === status.commandId,
    )
    const blocks = subagentId === undefined
      ? conversation.blocks
      : conversation.subagents.find((agent) => agent.id === subagentId)?.blocks ?? []
    const record: TerminalRecord = {
      id: status.commandId,
      // The transcript names the command by its script; the shell id is the fallback.
      name: current?.name ?? callScript(blocks, status.toolUseId) ?? status.shellId,
      phase: status.status,
      startedAt:
        current?.startedAt ?? new Date(Date.now() - status.runningMs).toISOString(),
      ...(status.status !== 'running'
        ? { endedAt: current?.endedAt ?? new Date().toISOString() }
        : {}),
      output: followLiveOutput(current?.output ?? '', current?.chars, status.tail, status.chars),
      chars: status.chars,
      toolUseId: status.toolUseId,
      ...(subagentId === undefined ? {} : { subagentId }),
    }
    if (current) {
      Object.assign(current, record)
    } else {
      conversation.terminals.push(record)
    }
  }
  updateLiveStatus(conversation)
}
