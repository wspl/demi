import { cloudSessionDirectory } from './execution-target'
import {
  clientFrameSchema,
  externalizeBlockMedia,
  type AgentServerTransport,
  type BlobStore,
  type ClientFrame,
  type ServerFrame
} from '@demicodes/agent'
import type { Block, ModelSelection } from '@demicodes/core'
import { errorMessage, SerialQueue } from '@demicodes/utils'
import type { ControlService, ConversationRecord } from '../storage/control'
import { resolveAttachmentRefs } from './attachment-refs'
import {
  conversationClientFrameSchema,
  type ConversationClientFrame
} from './client-frames'

/**
 * Scopes an incoming stream to its conversation: the session id and cwd are
 * resolved server-side from the conversation record (the browser never names
 * a cwd — each conversation uses its resolved Cloud, device or workspace path), the first user message becomes the
 * default title, attachment references inflate to inline bytes, activity
 * bumps the index row, and outbound transcript frames carry media by
 * reference (`backend.md` § Media by reference): every inline source becomes
 * `{ type: 'ref', ref, mediaType }`, the same form the block rows hold, and
 * the page fetches `GET /api/blobs/:sha256`.
 */
export interface ConversationTransportOptions {
  control: ControlService
  /**
   * Whether the conversation's user may name this provider — the same rule as
   * the PATCH route's.
   */
  providerAllowed: (providerId: string) => Promise<boolean>
  resolveRemoteFiles?: (
    conversation: ConversationRecord,
    content: unknown[]
  ) => Promise<unknown[]>
  admitFrame?: () => (() => void) | null
  modelSelection?: (
    providerId: string,
    selection: ModelSelection
  ) => Promise<ModelSelection>
  /**
   * Where every session of the conversation opens; the conversation’s Cloud
   * session directory or selected target path.
   */
  cwd?: string
  blobs?: BlobStore
}

/**
 * A frame the conversation refuses; its code goes back to the client as the
 * error frame's code.
 */
class FrameRefused extends Error {
  constructor(readonly code: string, message: string) {
    super(message)
  }
}

export function conversationScopedTransport(
  inner: AgentServerTransport,
  conversation: ConversationRecord,
  options: ConversationTransportOptions,
): AgentServerTransport {
  const { blobs } = options
  const cwd = options.cwd ?? cloudSessionDirectory(conversation.id)
  // Frame rewrites can await storage (attachment resolution inbound, blob
  // puts outbound); one chain per direction keeps delivery in arrival order.
  const deliveries = new SerialQueue()
  const sends = new SerialQueue()
  let closed = false
  const reportError = (code: string, message: string) => {
    if (closed)
      return
    try {
      inner.send({ type: 'error', code, message })
    } catch {
      closed = true
      inner.close()
    }
  }
  return {
    send: (frame) => {
      void sends.run(async () => {
        if (closed)
          return
        const outbound = blobs
          ? await externalizeFrameMedia(frame, blobs)
          : frame
        if (!closed)
          inner.send(outbound)
      }).catch((error: unknown) => reportError(
        'frame_send_failed',
        errorMessage(error)
      ))
    },
    onFrame: (handler) => {
      let subscribed = true
      const unsubscribe = inner.onFrame((frame) => {
        void deliveries.run(async () => {
          if (closed || !subscribed)
            return
          const parsed = conversationClientFrameSchema.safeParse(frame)
          if (!parsed.success) {
            reportError(
              'invalid_frame',
              `Invalid client frame: ${parsed.error.issues[0]?.message ?? 'invalid shape'}`
            )
            return
          }
          const release = options.admitFrame ? options.admitFrame() : () => {}
          if (!release)
            throw new FrameRefused(
              'conversation_busy',
              'A conversation operation is still running'
            )
          try {
            const current = await options.control.getConversation(
              conversation.id
            )
            if (!current || (current.archived && parsed.data.type !== 'close'))
              throw new FrameRefused(
                'archived',
                'Restore the conversation before writing to it'
              )
            const rewritten = await rewriteFrame(
              parsed.data,
              current,
              options,
              cwd
            )
            if (!closed && subscribed)
              await handler(rewritten)
          } finally {
            release()
          }
        }).catch((error: unknown) => {
          const parsed = conversationClientFrameSchema.safeParse(frame)
          if (parsed.success && parsed.data.type === 'edit_and_send') {
            if (!closed) {
              try {
                inner.send({
                  type: 'edit_result',
                  operationId: parsed.data.request.operationId,
                  status: 'rejected',
                  reason: errorMessage(error),
                })
              } catch {
                closed = true
                inner.close()
              }
            }
            return
          }
          reportError(
            error instanceof FrameRefused ? error.code : 'frame_delivery_failed',
            errorMessage(error),
          )
        })
      })
      return () => {
        subscribed = false
        unsubscribe()
      }
    },
    close: () => {
      closed = true
      inner.close()
    },
  }
}

/** Media in the frames that carry transcript blocks leaves as references. */
async function externalizeFrameMedia(
  frame: ServerFrame,
  blobs: BlobStore
): Promise<ServerFrame> {
  const externalize = (blocks: Block[]) => Promise.all(
    blocks.map((block) => externalizeBlockMedia(block, blobs))
  )
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
            if (patch.op === 'add' || patch.op === 'replace_block')
              return {
                ...patch,
                value: await externalizeBlockMedia(patch.value, blobs)
              }
            if (patch.op === 'replace')
              return {
                ...patch,
                value: await externalize(patch.value)
              }
            return patch
          }),
        ),
      }
    default:
      return frame
  }
}

async function rewriteFrame(
  frame: ConversationClientFrame,
  conversation: ConversationRecord,
  options: ConversationTransportOptions,
  cwd: string,
): Promise<ClientFrame> {
  const { control, blobs } = options
  const recordProvider = async (provider: {
    providerId: string;
    model: ModelSelection
  }) => {
    if (!(await options.providerAllowed(provider.providerId)))
      throw new FrameRefused('provider_not_found', 'No such provider')
    const model = options.modelSelection
      ? await options.modelSelection(provider.providerId, provider.model)
      : provider.model
    await control.setConversationModel(
      conversation.id,
      provider.providerId,
      model.model.id
    )
    return { ...provider, model }
  }
  if (frame.type === 'open') {
    const provider = await recordProvider(frame.provider)
    return { ...frame, provider, sessionId: conversation.id, cwd }
  }
  if (frame.type === 'send' || frame.type === 'steer') {
    let content: unknown[] = options.resolveRemoteFiles
      ? await options.resolveRemoteFiles(conversation, frame.content)
      : frame.content
    if (blobs)
      content = await resolveAttachmentRefs(
        { control, blobs, userId: conversation.userId },
        content
      )
    const rewritten = clientFrameSchema.parse({ ...frame, content })
    if (frame.type === 'send') {
      const text = frame.content.flatMap(block => block.type === 'text'
        ? [block.text]
        : [])[0]
      const title = (text ?? '').replace(/\s+/g, ' ').trim().slice(0, 80)
      if (title)
        await control.defaultConversationTitle(conversation.id, title)
      await control.touchConversation(conversation.id)
    }
    return rewritten
  }
  if (frame.type === 'set_provider') {
    return { ...frame, provider: await recordProvider(frame.provider) }
  }
  return frame
}
