import { cloudSessionDirectory } from './execution-target'
import {
  clientFrameSchema,
  externalizeBlockMedia,
  type AgentServerTransport,
  type BlobStore,
  type ClientFrame,
  type ServerFrame,
  type TranscriptPatch
} from '@demicodes/agent'
import type { Block, ModelSelection } from '@demicodes/core'
import { errorMessage, SerialQueue } from '@demicodes/utils'
import type { ProviderSelection } from '@demicodes/provider'
import type { ControlService, ConversationRecord } from '../storage/control'
import { titleFromMessage } from './title'
import type { FailureFactsReader } from './failure-facts'
import { resolveUploadRefs } from './attachment-refs'
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
  admitFrame?: (signal: AbortSignal) => Promise<() => void>
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
  /** Puts an attachment's bytes on the conversation's host; returns the absolute path. */
  writeAttachment?: (fileName: string, data: Uint8Array) => Promise<string>
  /**
   * The send that gave the conversation its message-derived title: a generated
   * title may follow (`product.md` § Conversation titles).
   */
  startTitle?: (provider: ProviderSelection, text: string) => void
  /** Reads the failure facts sent beside the transcript frames (`backend.md` § Failure facts). */
  readFailures?: FailureFactsReader
}

/** What the connection has established so far; a send reads the provider an open or switch set. */
interface ConnectionState {
  provider: ProviderSelection | null
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
  const cwd = options.cwd
    ?? (conversation.target.kind === 'cloud' ? conversation.target.path : undefined)
    ?? cloudSessionDirectory(conversation.id)
  // Frame rewrites can await storage (attachment resolution inbound, blob
  // puts outbound); one chain per direction keeps delivery in arrival order.
  const deliveries = new SerialQueue()
  const sends = new SerialQueue()
  const closing = new AbortController()
  const connection: ConnectionState = { provider: null }
  const close = () => {
    closing.abort()
    inner.close()
  }
  const reportError = (code: string, message: string) => {
    if (closing.signal.aborted)
      return
    try {
      inner.send({ type: 'error', code, message })
    } catch {
      close()
    }
  }
  return {
    send: (frame) => {
      void sends.run(async () => {
        if (closing.signal.aborted)
          return
        const outbound = await presentFrame(frame, options)
        if (!closing.signal.aborted)
          inner.send(outbound)
      }).catch((error: unknown) => reportError(
        'frame_send_failed',
        errorMessage(error)
      ))
    },
    onFrame: (handler) => {
      const subscription = new AbortController()
      const signal = AbortSignal.any([closing.signal, subscription.signal])
      const unsubscribe = inner.onFrame((frame) => {
        void deliveries.run(async () => {
          if (signal.aborted)
            return
          const parsed = conversationClientFrameSchema.safeParse(frame)
          if (!parsed.success) {
            reportError(
              'invalid_frame',
              `Invalid client frame: ${parsed.error.issues[0]?.message ?? 'invalid shape'}`
            )
            return
          }
          const release = options.admitFrame ? await options.admitFrame(signal) : () => {}
          try {
            if (signal.aborted)
              return
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
              cwd,
              connection
            )
            if (!signal.aborted)
              await handler(rewritten)
          } finally {
            release()
          }
        }).catch((error: unknown) => {
          if (signal.aborted)
            return
          const parsed = conversationClientFrameSchema.safeParse(frame)
          if (parsed.success && parsed.data.type === 'edit_and_send') {
            try {
              inner.send({
                type: 'edit_result',
                operationId: parsed.data.request.operationId,
                status: 'rejected',
                reason: errorMessage(error),
              })
            } catch {
              close()
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
        subscription.abort()
        unsubscribe()
      }
    },
    close,
  }
}

/**
 * A server frame as the browser gets it: media as references, and the failure
 * facts of the error blocks it carries beside them.
 */
async function presentFrame(
  frame: ServerFrame,
  options: ConversationTransportOptions
): Promise<ServerFrame> {
  const outbound = options.blobs
    ? await externalizeFrameMedia(frame, options.blobs)
    : frame
  if (!options.readFailures)
    return outbound
  return attachFailures(outbound, options.readFailures)
}

/**
 * A transcript frame with the facts of the error blocks it carries: a reset's
 * blocks, or the blocks its patches add. Unchanged when they yield none.
 */
async function attachFailures(
  frame: ServerFrame,
  readFailures: FailureFactsReader
): Promise<ServerFrame> {
  switch (frame.type) {
    case 'transcript_reset':
    case 'subagent_transcript_reset': {
      const failures = await readFailures(frame.blocks)
      return Object.keys(failures).length > 0 ? { ...frame, failures } : frame
    }
    case 'transcript_patch':
    case 'subagent_transcript_patch': {
      const failures = await readFailures(blocksAdded(frame.patches))
      return Object.keys(failures).length > 0 ? { ...frame, failures } : frame
    }
    default:
      return frame
  }
}

/** The whole blocks a list of patches puts into a transcript. */
function blocksAdded(patches: TranscriptPatch[]): Block[] {
  return patches.flatMap((patch) => {
    if (patch.op === 'add' || patch.op === 'replace_block')
      return [patch.value]
    if (patch.op === 'replace')
      return patch.value
    return []
  })
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
  connection: ConnectionState,
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
    connection.provider = { providerId: provider.providerId, model }
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
    if (blobs && options.writeAttachment)
      content = await resolveUploadRefs(
        {
          control,
          blobs,
          userId: conversation.userId,
          writeToHost: options.writeAttachment
        },
        content
      )
    const rewritten = clientFrameSchema.parse({ ...frame, content })
    if (frame.type === 'send') {
      const text = frame.content.flatMap(block => block.type === 'text'
        ? [block.text]
        : [])[0] ?? ''
      const title = titleFromMessage(text)
      const titled = title
        ? await control.defaultConversationTitle(conversation.id, title)
        : false
      if (titled && connection.provider)
        options.startTitle?.(connection.provider, text)
      await control.touchConversation(conversation.id)
    }
    return rewritten
  }
  if (frame.type === 'set_provider') {
    return { ...frame, provider: await recordProvider(frame.provider) }
  }
  return frame
}
