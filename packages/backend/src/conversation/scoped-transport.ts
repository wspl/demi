import { externalizeBlockMedia, type AgentServerTransport, type BlobStore, type ClientFrame, type ServerFrame } from '@demicodes/agent'
import type { Block } from '@demicodes/core'
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
 * default title, attachment references inflate to inline bytes, activity
 * bumps the index row, and outbound transcript frames carry media by
 * reference (`backend.md` § Media by reference): every inline source becomes
 * `{ type: 'ref', ref, mediaType }`, the same form the block rows hold, and
 * the page fetches `GET /api/blobs/:sha256`.
 */
export function conversationScopedTransport(
  inner: AgentServerTransport,
  conversation: ConversationRecord,
  control: ControlService,
  cwd: string = HOSTLESS_HOME,
  blobs?: BlobStore,
): AgentServerTransport {
  // Frame rewrites can await storage (attachment resolution inbound, blob
  // puts outbound); one chain per direction keeps delivery in arrival order.
  let deliveries: Promise<void> = Promise.resolve()
  let sends: Promise<void> = Promise.resolve()
  return {
    send: (frame) => {
      if (!blobs) {
        inner.send(frame)
        return
      }
      sends = sends.then(async () => inner.send(await externalizeFrameMedia(frame, blobs)))
    },
    onFrame: (handler) =>
      inner.onFrame((frame) => {
        deliveries = deliveries.then(async () => {
          handler(await rewriteFrame(frame, conversation, control, cwd, blobs))
        })
      }),
    close: () => inner.close(),
  }
}

/** Media in the frames that carry transcript blocks leaves as references. */
async function externalizeFrameMedia(frame: ServerFrame, blobs: BlobStore): Promise<ServerFrame> {
  const externalize = (blocks: Block[]) => Promise.all(blocks.map((block) => externalizeBlockMedia(block, blobs)))
  switch (frame.type) {
    case 'transcript_reset':
    case 'subagent_transcript_reset':
      return { ...frame, blocks: await externalize(frame.blocks) }
    case 'transcript_patch':
    case 'subagent_transcript_patch':
      return {
        ...frame,
        patches: await Promise.all(
          frame.patches.map(async (patch) => {
            if (patch.op === 'add' || patch.op === 'replace_block') return { ...patch, value: await externalizeBlockMedia(patch.value, blobs) }
            if (patch.op === 'replace') return { ...patch, value: await externalize(patch.value) }
            return patch
          }),
        ),
      }
    default:
      return frame
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
