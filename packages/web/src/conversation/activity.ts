import {
  EMPTY_TRANSCRIPT,
  applyTranscriptPatches,
  heldBlocks,
  resetTranscript,
  type ClientSessionEvent,
} from '@demicodes/web-ui/transport/protocol'
import { latestBlocks } from '@demicodes/web-ui/agent/history'
import type { Conversation } from '../state/types'
import { findShellCall } from './terminals'
import { isConversationActive } from '@demicodes/web-ui/agent/conversation-status'
import { followFrame } from '@demicodes/web-ui/agent/terminals'

/** Map agent events to the product's child and terminal presentation records. */
export function updateLiveStatus(conversation: Conversation): void {
  if (isConversationActive(conversation.phase, conversation.subagents)) {
    conversation.status = 'active'
    return
  }
  const last = latestBlocks(conversation.history).at(-1)
  conversation.status =
    conversation.lastError || last?.type === 'error'
      ? 'error'
      : last?.type === 'abort'
        ? 'aborted'
        : conversation.history.length
          ? 'done'
          : 'idle'
}

export function applyConversationEvent(
  conversation: Conversation,
  event: ClientSessionEvent,
): void {
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
        number: job.number,
        name: job.description,
        phase: job.phase,
        startedAt: job.startedAt,
        endedAt: job.endedAt ?? undefined,
        history: EMPTY_TRANSCRIPT,
        pendingCalls: [],
        waitingSince: null,
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
        child.history = resetTranscript(child.history, event)
      } else {
        child.history = applyTranscriptPatches(child.history, event.patches)
      }
      child.failures = { ...child.failures, ...event.failures }
    }
  } else if (event.type === 'pending_calls' && event.subagentId !== undefined) {
    const child = conversation.subagents.find((agent) => agent.id === event.subagentId)
    if (child) {
      child.pendingCalls = event.pendingCalls
    }
  } else if (event.type === 'waiting' && event.subagentId !== undefined) {
    const child = conversation.subagents.find((agent) => agent.id === event.subagentId)
    if (child) {
      child.waitingSince = event.since
    }
  } else if (event.type === 'shell_output') {
    // A command's live view, the same for every page (`runtime.md` § Live
    // output): what it adds to the output shown, under its call while the
    // call runs and in the dock after.
    const { subagentId, status } = event
    const current = conversation.terminals.find(
      (terminal) => terminal.id === status.commandId,
    )
    // The call may lie in any window the page holds, not only the latest.
    const blocks = heldBlocks(subagentId === undefined
      ? conversation.history
      : conversation.subagents.find((agent) => agent.id === subagentId)?.history ?? EMPTY_TRANSCRIPT)
    // The call that started it names it; the command's id stands in until the
    // page has the call. Until the record has the call's script, each frame
    // looks for the call again: the transcript may not have held it yet, or
    // held it light, without a long script, which the store then reads whole
    // (`web-api.md` § Light form). A record with its script keeps its call,
    // which a later call that reuses the id does not rename.
    const call = current?.script !== undefined
      ? current
      : findShellCall(blocks, status.toolUseId) ?? current
    const record = followFrame(current, status, call, subagentId, Date.now())
    if (current) {
      Object.assign(current, record)
    } else {
      conversation.terminals.push(record)
    }
  }
  updateLiveStatus(conversation)
}
