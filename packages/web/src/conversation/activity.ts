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
import { followFrame, isTerminalRunning } from '@demicodes/web-ui/agent/terminals'
import { subagentJobFields } from '@demicodes/web-ui/agent/subagents'

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
    const fields = subagentJobFields(event.job)
    const existing = conversation.subagents.find(
      (agent) => agent.id === fields.id,
    )
    if (existing) {
      // A resume's `started` job is a new round: its start, its end and
      // what it runs on replace the closed round's.
      Object.assign(existing, fields)
      if (event.event === 'closed') {
        // A closed child runs nothing more; a reopened page learns of the
        // close without the frames that ended its work before it.
        existing.sessionPhase = 'idle'
        existing.waitingSince = null
        existing.pendingCalls = []
      }
    } else {
      conversation.subagents.push({
        ...fields,
        // Its `phase` frame follows the `started` one.
        sessionPhase: 'idle',
        history: EMPTY_TRANSCRIPT,
        pendingCalls: [],
        waitingSince: null,
        failures: {},
        contextUsage: null,
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
  } else if (event.type === 'phase' && event.subagentId !== undefined) {
    const child = conversation.subagents.find((agent) => agent.id === event.subagentId)
    if (child) {
      child.sessionPhase = event.phase
    }
  } else if (event.type === 'context_usage') {
    // The root's usage is the runtime's; no child has the root's id.
    const child = conversation.subagents.find((agent) => agent.id === event.node)
    if (child) {
      child.contextUsage = event.usage
    }
  } else if (event.type === 'waiting' && event.subagentId !== undefined) {
    const child = conversation.subagents.find((agent) => agent.id === event.subagentId)
    if (child) {
      child.waitingSince = event.since
    }
  } else if (event.type === 'opened') {
    // The open's snapshot brings every running command, and the page's
    // running commands are those (`web-application.md` § What a reopened
    // socket brings back): one that ended or went while the page was away
    // shows so, and one that still runs comes back with its next frame.
    conversation.terminals = conversation.terminals.filter((terminal) => !isTerminalRunning(terminal.phase))
  } else if (event.type === 'subagents') {
    // Every child as the open's snapshot lists it, which the page takes
    // whole (`subagents.md` § Protocol). A running child's frames follow
    // with its state and its latest page; a finished one keeps the
    // transcript the page holds while its revision has not moved, and is
    // read again when shown otherwise.
    const held = new Map(conversation.subagents.map((agent) => [agent.id, agent]))
    conversation.subagents = event.subagents.map((summary) => {
      const agent = held.get(summary.subagentId)
      const current = agent !== undefined && (summary.phase === 'running' || agent.revision === summary.revision)
      return {
        ...subagentJobFields(summary),
        revision: summary.revision,
        sessionPhase: 'idle',
        waitingSince: null,
        pendingCalls: [],
        history: current ? agent.history : EMPTY_TRANSCRIPT,
        failures: current ? agent.failures : {},
        contextUsage: agent?.contextUsage ?? null,
      }
    })
  } else if (event.type === 'shell_output') {
    // A command's live view, the same for every page (`shell-tool.md` § Live
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
