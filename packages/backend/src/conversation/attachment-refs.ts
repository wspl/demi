import type { BlobStore } from '@demicodes/agent'
import type { UserContentBlock } from '@demicodes/core'
import { z } from 'zod'
import type { ControlService } from '../storage/control'

/**
 * The wire form of an uploaded attachment inside a `send` frame: media blocks
 * whose source is `{type:'ref', ref: <attachment id>}` (documents add the
 * fileName the table does not store). A backend extension of the frame
 * protocol, resolved to inline bytes here — before the agent-server boundary
 * validates the frame — so providers and the session only ever see the core
 * media set.
 */
const refSourceSchema = z.object({
  type: z.literal('ref'),
  ref: z.string().min(1),
  fileName: z.string().optional(),
})

export interface AttachmentRefDeps {
  control: ControlService
  blobs: BlobStore
  userId: string
}

export async function resolveAttachmentRefs(deps: AttachmentRefDeps, content: unknown[]): Promise<unknown[]> {
  return Promise.all(content.map((block) => resolveBlock(deps, block)))
}

async function resolveBlock(deps: AttachmentRefDeps, block: unknown): Promise<unknown> {
  if (typeof block !== 'object' || block === null) return block
  const candidate = block as { type?: unknown; source?: unknown }
  if (candidate.type !== 'image' && candidate.type !== 'video' && candidate.type !== 'document') return block
  const parsed = refSourceSchema.safeParse(candidate.source)
  if (!parsed.success) return block

  const attachment = await deps.control.getAttachment(parsed.data.ref)
  if (!attachment || attachment.userId !== deps.userId) return missing(parsed.data.ref)
  const data = await deps.blobs.get(attachment.sha256)
  if (!data) return missing(parsed.data.ref)

  if (candidate.type === 'document') {
    const source = { data, mediaType: attachment.mediaType, fileName: parsed.data.fileName ?? parsed.data.ref }
    return { type: 'document', source } satisfies UserContentBlock
  }
  const source = { type: 'binary', data, mediaType: attachment.mediaType } as const
  return { type: candidate.type, source } as UserContentBlock
}

function missing(ref: string): UserContentBlock {
  return { type: 'text', text: `[attachment ${ref} is not available]` }
}
