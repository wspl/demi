import { ConversationClient } from '@demicodes/conversation-client'
import type { AgentEntry, Block, ClientFrame, ModelSelection, ServerFrame, SessionPhase } from '@demicodes/protocol'
import { NO_COST } from '../context-cost'

export const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'stub',
    name: 'Stub',
    contextWindow: 1000,
    outputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}

export const createdAt = '2026-09-24T12:00:00.000Z'

/** The root's entry of the conversation `id`, its session in `phase`. */
export function rootEntry(id: string, phase: SessionPhase = 'idle', parts: Partial<AgentEntry> = {}): AgentEntry {
  return { id, role: { kind: 'root' }, phase, waitedMs: null, pendingCalls: [], context: null, cost: NO_COST, queue: [], pendingSteers: [], starting: null, ...parts }
}

/** The entry of the running child `id` of the root `parent`, its session in `phase`. */
export function subagentEntry(id: string, parent: string, phase: SessionPhase = 'idle', parts: Partial<AgentEntry> = {}): AgentEntry {
  return {
    ...rootEntry(id, phase),
    role: {
      kind: 'subagent',
      number: 1,
      parent,
      description: 'Read the notes',
      profile: null,
      model: { providerId: 'stub', provider: 'Stub', model: 'Stub', effort: null, fast: false },
      round: 1,
      forkedFrom: null,
      state: 'running',
      startedAt: createdAt,
      endedAt: null,
      cwd: '/Users/alex',
      onPath: true,
    },
    ...parts,
  }
}

/** The conversation's agents, `entries`, whole at `revision` of the epoch `e1`. */
export function agentsFrame(entries: AgentEntry[], revision = 1): ServerFrame {
  return { type: 'agents', version: { epoch: 'e1', revision }, since: null, agents: entries }
}

export function userBlock(id: string, turnId: string, text: string): Extract<Block, { type: 'user' }> {
  return { type: 'user', id, turnId, createdAt, model, content: [{ type: 'text', text }], preamble: null }
}

/**
 * A client over a transport the test drives, as a conversation socket that
 * answers `open` with `opened`, or with `answer`: what the client sent, and a
 * way to deliver what the server would send.
 */
export function clientHarness(answer: ServerFrame = { type: 'opened' }) {
  const sent: ClientFrame[] = []
  let deliver: (frame: unknown) => void = () => {}
  let closes = 0
  const client = new ConversationClient({
    send(frame) {
      sent.push(frame)
      if (frame.type === 'open') {
        deliver(answer)
      }
    },
    onFrame(handler) {
      deliver = handler
      return () => {
        deliver = () => {}
      }
    },
    onClose() {
      return () => {}
    },
    close() {
      closes += 1
    },
  })
  return {
    client,
    sent,
    receive: (frame: ServerFrame) => deliver(frame),
    closes: () => closes,
  }
}
