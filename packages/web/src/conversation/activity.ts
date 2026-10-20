import {
  EMPTY_TRANSCRIPT,
  applyTranscriptPatches,
  resetTranscript,
  type ClientSessionEvent,
} from '@demicodes/web-ui/transport/protocol'
import type { AgentEntry, Version } from '@demicodes/protocol'
import { latestBlocks } from '@demicodes/web-ui/agent/history'
import type { Conversation, SideRecord } from '../state/types'
import { sideRecord } from './side-records'
import { isConversationActive } from '@demicodes/web-ui/agent/conversation-status'
import { followFrame, isTerminalRunning } from '@demicodes/web-ui/agent/terminals'
import { subagentEntryFields } from '@demicodes/web-ui/agent/subagents'

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

/**
 * Takes the conversation's agents as `entries` say them at `version`, as
 * they arrived at `receivedAt`: `whole` when they are every agent, as an
 * open's answer without the changes since a version lists them, so an agent
 * they leave out is gone. Each record keeps what the page holds of its
 * transcript; the root's entry is the runtime's, but for its cost.
 */
export function takeAgents(
  conversation: Conversation,
  entries: readonly AgentEntry[],
  version: Version,
  receivedAt: number,
  whole: boolean,
): void {
  const children = new Map(conversation.subagents.map((agent) => [agent.id, agent]))
  const sides = new Map(conversation.sides.map((side) => [side.id, side]))
  const listedChildren = whole ? [] : [...conversation.subagents]
  const listedSides = whole ? conversation.sides.filter((side) => !side.listed) : [...conversation.sides]
  for (const entry of entries) {
    const role = entry.role
    if (role.kind === 'root') {
      conversation.cost = entry.cost
    } else if (role.kind === 'subagent') {
      const fields = subagentEntryFields(entry, role, receivedAt)
      const held = children.get(entry.id)
      if (held) {
        Object.assign(held, fields)
        if (!listedChildren.includes(held)) {
          listedChildren.push(held)
        }
      } else {
        listedChildren.push({ ...fields, history: EMPTY_TRANSCRIPT, failures: {} })
      }
    } else {
      const { kind: _kind, ...side } = role
      const held = sides.get(entry.id) ?? sideRecord({ id: entry.id, ...side })
      held.listed = true
      // A fold on its way stays as the user chose it.
      held.record = { id: entry.id, ...side, folded: held.folding ? held.folding.intent : side.folded }
      held.sessionPhase = entry.phase
      held.pendingCalls = entry.pendingCalls
      held.waitingSince = entry.waitedMs === null ? null : receivedAt - entry.waitedMs
      if (!listedSides.includes(held)) {
        listedSides.push(held)
      }
    }
  }
  conversation.subagents = listedChildren
  conversation.sides = listedSides
  conversation.agentsHeld = version
}

/** The record of the node `node` other than the root, whose messages it holds. */
function nodeRecord(conversation: Conversation, node: string): Conversation['subagents'][number] | SideRecord | undefined {
  return conversation.subagents.find((agent) => agent.id === node) ?? conversation.sides.find((side) => side.id === node)
}

export function applyConversationEvent(
  conversation: Conversation,
  event: ClientSessionEvent,
): void {
  if (event.type === 'agents') {
    takeAgents(conversation, event.agents, event.version, event.receivedAt, event.since === undefined)
  } else if (event.type === 'agent') {
    // The client checked the revision follows the one it holds, which
    // came with the open's agents.
    const held = conversation.agentsHeld
    if (held) {
      takeAgents(conversation, [event.agent], { epoch: held.epoch, revision: event.revision }, event.receivedAt, false)
    }
  } else if (event.type === 'transcript_reset' || event.type === 'transcript_patch') {
    // The root's messages are the runtime's.
    const record = event.node === conversation.id ? undefined : nodeRecord(conversation, event.node)
    if (record) {
      record.history = event.type === 'transcript_reset'
        ? resetTranscript(record.history, event, event.asked)
        : applyTranscriptPatches(record.history, event.patches, performance.now(), event.version)
      record.failures = { ...record.failures, ...event.failures }
    }
  } else if (event.type === 'opened') {
    // The open's snapshot brings every running command of the nodes the
    // page shows, and the page's running commands are those: one that ended
    // or went while the page was away shows so, and one that still runs
    // comes back with its next frame.
    conversation.terminals = conversation.terminals.filter((terminal) => !isTerminalRunning(terminal.phase))
  } else if (event.type === 'shell_output') {
    // A command's live view, the same for every page that shows its node
    // (`crates/agent-server/src/server/tree/live.rs`): what it adds to the
    // output shown, under its call while the call runs and in the dock
    // after. The frame names the command by its call's title and script,
    // which its record keeps, so its tab needs no call the page holds.
    const { node, status } = event
    const current = conversation.terminals.find(
      (terminal) => terminal.id === status.commandId,
    )
    const record = followFrame(current, status, node === conversation.id ? undefined : node, Date.now())
    if (current) {
      Object.assign(current, record)
    } else {
      conversation.terminals.push(record)
    }
  }
  updateLiveStatus(conversation)
}
