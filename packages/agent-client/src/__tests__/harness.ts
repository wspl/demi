import type { Block, ClientFrame, ModelSelection, PendingSteer, ServerFrame } from '@demicodes/protocol'
import { AgentClient } from '../client'
import type { ClientSessionEvent } from '../events'
import type { AgentClientTransport } from '../transport'

export const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'stub',
    name: 'Stub',
    contextWindow: 1000,
    inputLimit: null,
    outputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}

export const createdAt = '2026-09-24T12:00:00.000Z'

export function text(id: string, value: string): Block {
  return { type: 'text', id, createdAt, model, text: value }
}

export function user(id: string, turnId: string, value: string): Block {
  return { type: 'user', id, turnId, createdAt, model, content: [{ type: 'text', text: value }], preamble: null }
}

export function pendingSteer(id: string): PendingSteer {
  return { id, turnId: 'turn-1', model, content: [{ type: 'text', text: 'identical guidance' }] }
}

/** A client over a transport the test drives: what the client sends, and what the server answers. */
export function harness() {
  const sent: ClientFrame[] = []
  const events: ClientSessionEvent[] = []
  let deliver: (frame: unknown) => void = () => {}
  let end: (error: Error) => void = () => {}
  let closes = 0
  const transport: AgentClientTransport = {
    send: (frame) => {
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
  const client = new AgentClient(transport)
  client.subscribe((event) => events.push(event))
  return {
    client,
    sent,
    events,
    /** Delivers a frame as the server would send it. */
    receive: (frame: ServerFrame) => deliver(frame),
    /** Delivers any JSON value, such as one the contract does not allow. */
    receiveValue: (value: unknown) => deliver(value),
    /** Ends the connection from the server's side. */
    end: (error: Error) => end(error),
    closes: () => closes,
  }
}

/** Lets the promise callbacks that are due run. */
export async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}
