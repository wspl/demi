import type { AgentServerTransport, BlobStore, ClientFrame } from '@demicodes/agent'
import type { ControlService, ConversationRecord } from '../storage/control'
import { resolveAttachmentRefs } from './attachment-refs'

/** Virtual working directory every virtual-target conversation starts in. */
/** The hostless home: cwd of every hostless shell, and where the model's files live (`sessions-and-targets.md` § The namespace). */
export const HOSTLESS_HOME = '/home/demi'
/** The two subtrees a hostless script may touch, plus `/dev/null`. */
export const HOSTLESS_NAMESPACE: readonly string[] = [HOSTLESS_HOME, '/tmp']

/**
 * Scopes an incoming stream to its conversation: the session id and cwd are
 * resolved server-side from the conversation record (the browser never names
 * a cwd — a workspace-bound conversation runs in its workspace path, a
 * virtual one in the virtual constant), the first user message becomes the
 * default title, attachment references inflate to inline bytes, and activity
 * bumps the index row.
 */
export function conversationScopedTransport(
  inner: AgentServerTransport,
  conversation: ConversationRecord,
  control: ControlService,
  cwd: string = HOSTLESS_HOME,
  blobs?: BlobStore,
): AgentServerTransport {
  // Frame rewrites can await storage (attachment resolution); the chain keeps
  // delivery in arrival order regardless.
  let deliveries: Promise<void> = Promise.resolve()
  return {
    send: (frame) => inner.send(frame),
    onFrame: (handler) =>
      inner.onFrame((frame) => {
        deliveries = deliveries.then(async () => {
          handler(await rewriteFrame(frame, conversation, control, cwd, blobs))
        })
      }),
    close: () => inner.close(),
  }
}

async function rewriteFrame(
  frame: ClientFrame,
  conversation: ConversationRecord,
  control: ControlService,
  cwd: string,
  blobs: BlobStore | undefined,
): Promise<ClientFrame> {
  if (frame.type === 'open') {
    void control.setConversationModel(conversation.id, frame.provider.providerId, frame.provider.model.model.id)
    return { ...frame, sessionId: conversation.id, cwd }
  }
  if (frame.type === 'send') {
    const text = frame.content.find((block): block is { type: 'text'; text: string } => block.type === 'text')?.text
    const title = (text ?? '').replace(/\s+/g, ' ').trim().slice(0, 80)
    if (title) void control.defaultConversationTitle(conversation.id, title)
    void control.touchConversation(conversation.id)
    if (!blobs) return frame
    const content = (await resolveAttachmentRefs(
      { control, blobs, userId: conversation.userId },
      frame.content,
    )) as typeof frame.content
    return { ...frame, content }
  }
  if (frame.type === 'set_provider') {
    void control.setConversationModel(conversation.id, frame.provider.providerId, frame.provider.model.model.id)
    return frame
  }
  return frame
}
