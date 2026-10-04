import { ConversationClient } from '@demicodes/conversation-client'
import type { Block, ClientFrame, ModelSelection, ServerFrame } from '@demicodes/protocol'

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
