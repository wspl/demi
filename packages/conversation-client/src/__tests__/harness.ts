import type { AgentEntry, Block, ClientFrame, ModelSelection, PendingCall, PendingSteer, ServerFrame, SessionPhase } from '@demicodes/protocol'
import { ConversationClient } from '../client'
import type { ClientSessionEvent } from '../events'
import { EMPTY_TRANSCRIPT, applyTranscriptPatches, heldBlocks, resetTranscript, type HeldTranscript } from '../transcript'
import type { ConversationClientTransport } from '../transport'

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

/** The root's node, the conversation's id. */
export const ROOT = 'conversation'

const NO_COST: AgentEntry['cost'] = {
  totals: {
    inputTokens: 0,
    cacheWriteTokens: 0,
    cacheReadTokens: 0,
    outputTokens: 0,
    price: { total: null, input: null, cacheWrite: null, cacheRead: null, output: null },
  },
  calls: 0,
}

/** The entry of the agent `id`, the root or a running child of it, its session in `phase`. */
export function entry(id: string, phase: SessionPhase, pendingCalls: PendingCall[] = []): AgentEntry {
  const role: AgentEntry['role'] = id === ROOT
    ? { kind: 'root' }
    : {
        kind: 'subagent',
        number: 1,
        parent: ROOT,
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
      }
  return { id, role, phase, waitedMs: null, pendingCalls, context: null, cost: NO_COST }
}

export function text(id: string, value: string): Block {
  return { type: 'text', id, createdAt, model, text: value }
}

export function user(id: string, turnId: string, value: string): Block {
  return { type: 'user', id, turnId, createdAt, model, content: [{ type: 'text', text: value }], preamble: null }
}

export function pendingSteer(id: string): PendingSteer {
  return { id, turnId: 'turn-1', model, content: [{ type: 'text', text: 'identical guidance' }] }
}

/**
 * A client over a transport the test drives: what the client sends, and what
 * the server answers. Unless `opened` is false, the client has opened the
 * conversation `ROOT`, which subscribes to its root, and `sent` starts empty.
 */
export function harness({ opened = true }: { opened?: boolean } = {}) {
  const sent: ClientFrame[] = []
  const events: ClientSessionEvent[] = []
  let deliver: (frame: unknown) => void = () => {}
  let end: (error: Error) => void = () => {}
  let closes = 0
  let refusal: Error | null = null
  const transport: ConversationClientTransport = {
    send: (frame) => {
      if (refusal) {
        throw refusal
      }
      sent.push(frame)
    },
    onFrame(handler) {
      deliver = handler
      return () => {
        deliver = () => {}
      }
    },
    onClose(handler) {
      end = handler
      return () => {
        end = () => {}
      }
    },
    close: () => {
      closes += 1
    },
  }
  const client = new ConversationClient(transport)
  // The root's transcript as the page holds it, from the client's events.
  let transcript: HeldTranscript = EMPTY_TRANSCRIPT
  client.subscribe((event) => {
    events.push(event)
    if ((event.type === 'transcript_reset' || event.type === 'transcript_patch') && event.node !== ROOT) {
      return
    }
    if (event.type === 'transcript_reset') {
      transcript = resetTranscript(transcript, event, event.asked)
    } else if (event.type === 'transcript_patch') {
      transcript = applyTranscriptPatches(transcript, event.patches)
    } else if (event.type === 'closed') {
      transcript = EMPTY_TRANSCRIPT
    }
  })
  if (opened) {
    // A wait the test does not answer fails when the client disconnects.
    client.open({ root: ROOT }).catch(() => {})
    sent.length = 0
  }
  // The agents' revision the server has sent; null before the first frame.
  let revision: number | null = null
  const entries = new Map<string, AgentEntry>()
  /** Sends the agent `node`'s entry as it changed, after the whole agents first. */
  const change = (agent: AgentEntry) => {
    entries.set(agent.id, agent)
    if (revision === null) {
      revision = 1
      deliver({ type: 'agents', version: { epoch: 'e', revision }, agents: [...entries.values()] })
    } else {
      revision += 1
      deliver({ type: 'agent', revision, agent })
    }
  }
  return {
    client,
    sent,
    events,
    /** Delivers the agent `node`'s session phase, as its entry's change. */
    phase: (phase: SessionPhase, node: string = ROOT) => change({ ...(entries.get(node) ?? entry(node, phase)), phase }),
    /** Delivers the calls the agent `node`'s model writes, as its entry's change. */
    calls: (pendingCalls: PendingCall[], node: string = ROOT) =>
      change({ ...(entries.get(node) ?? entry(node, 'running')), pendingCalls }),
    /** The root's blocks the page holds, from the client's events. */
    blocks: () => heldBlocks(transcript),
    /** Delivers a frame as the server would send it. */
    receive: (frame: ServerFrame) => deliver(frame),
    /** Delivers any JSON value, such as one the contract does not allow. */
    receiveValue: (value: unknown) => deliver(value),
    /** Ends the connection from the server's side. */
    end: (error: Error) => end(error),
    /** Makes every later send throw `error`, as a socket that cannot send does. */
    refuseSends: (error: Error) => {
      refusal = error
    },
    closes: () => closes,
  }
}

/** Lets the promise callbacks that are due run. */
export async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}
