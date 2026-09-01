import type { AgentServerTransport, ClientFrame } from '@demicodes/agent'
import type { ControlService, ConversationRecord } from '../storage/control'

/** Virtual working directory every virtual-target conversation starts in. */
export const VIRTUAL_WORKSPACE_CWD = '/workspace'

/**
 * Scopes an incoming stream to its conversation: the session id and cwd are
 * resolved server-side from the conversation record (the browser never names
 * a cwd — a workspace-bound conversation runs in its workspace path, a
 * virtual one in the virtual constant), the first user message becomes the
 * default title, and activity bumps the index row.
 */
export function conversationScopedTransport(
  inner: AgentServerTransport,
  conversation: ConversationRecord,
  control: ControlService,
  cwd: string = VIRTUAL_WORKSPACE_CWD,
): AgentServerTransport {
  return {
    send: (frame) => inner.send(frame),
    onFrame: (handler) =>
      inner.onFrame((frame) => {
        handler(rewriteFrame(frame, conversation, control, cwd))
      }),
    close: () => inner.close(),
  }
}

function rewriteFrame(
  frame: ClientFrame,
  conversation: ConversationRecord,
  control: ControlService,
  cwd: string,
): ClientFrame {
  if (frame.type === 'open') {
    void control.setConversationModel(conversation.id, frame.provider.providerId, frame.provider.model.model.id)
    return { ...frame, sessionId: conversation.id, cwd }
  }
  if (frame.type === 'send') {
    const text = frame.content.find((block): block is { type: 'text'; text: string } => block.type === 'text')?.text
    const title = (text ?? '').replace(/\s+/g, ' ').trim().slice(0, 80)
    if (title) void control.defaultConversationTitle(conversation.id, title)
    void control.touchConversation(conversation.id)
    return frame
  }
  if (frame.type === 'set_provider') {
    void control.setConversationModel(conversation.id, frame.provider.providerId, frame.provider.model.model.id)
    return frame
  }
  return frame
}
