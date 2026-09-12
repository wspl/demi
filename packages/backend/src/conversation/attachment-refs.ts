import type { BlobStore } from '@demicodes/agent'
import {
  attachmentSnippet,
  isTextAttachment,
  sniffModelMediaType,
  type UserContentBlock
} from '@demicodes/core'
import type { Host } from '@demicodes/shell'
import { z } from 'zod'
import type { ControlService } from '../storage/control'

/**
 * The wire form of an upload inside a `send` or `steer` frame:
 * `{ type: 'upload', ref: <attachment id>, fileName }`. A backend extension
 * of the frame protocol, resolved here before the agent-server boundary
 * validates the frame (`product.md` § Attachments):
 *
 * - The file is written into Demi's own directory on the host, never the
 *   working directory (`~/.demi/attachments/<conversation>/`), and the
 *   message gets one `attachment` content block: the single record of the
 *   file, which providers render as a tag naming it and its path. Nothing of
 *   the file's content rides in the message.
 * - Media the model reads natively (an image, a video, a PDF) also rides as
 *   its media block, right before the attachment block, because tools cannot
 *   show the model a picture.
 */
const safeFileName = z
  .string()
  .min(1)
  .max(255)
  .refine(
    (name) => !/[/\\\0]/.test(name) && name !== '.' && name !== '..',
    'Expected a file name without path separators',
  )

export const uploadRefBlockSchema = z.strictObject({
  type: z.literal('upload'),
  ref: z.string().min(1),
  fileName: safeFileName,
})

/** Under the host user's home: Demi's files never touch a working directory. */
export const ATTACHMENTS_DIR = '.demi/attachments'

export interface AttachmentRefDeps {
  control: ControlService
  blobs: BlobStore
  userId: string
  /** Puts the bytes on the conversation's host; returns the absolute path they landed at. */
  writeToHost: (fileName: string, data: Uint8Array) => Promise<string>
}

export async function resolveUploadRefs(
  deps: AttachmentRefDeps,
  content: unknown[]
): Promise<unknown[]> {
  const resolved: unknown[] = []
  for (const block of content) {
    resolved.push(...await resolveBlock(deps, block))
  }
  return resolved
}

async function resolveBlock(
  deps: AttachmentRefDeps,
  block: unknown
): Promise<unknown[]> {
  const parsed = uploadRefBlockSchema.safeParse(block)
  if (!parsed.success)
    return [block]
  const { ref, fileName } = parsed.data
  const attachment = await deps.control.getAttachment(ref)
  if (!attachment || attachment.userId !== deps.userId)
    return [missing(ref)]
  const data = await deps.blobs.get(attachment.sha256)
  if (!data)
    return [missing(ref)]

  const path = await deps.writeToHost(fileName, data)
  const record: UserContentBlock = {
    type: 'attachment',
    name: fileName,
    path,
    mediaType: attachment.mediaType,
    sizeBytes: attachment.sizeBytes,
    sha256: attachment.sha256,
  }
  if (isTextAttachment(fileName, attachment.mediaType)) {
    record.snippet = attachmentSnippet(new TextDecoder().decode(data.subarray(0, 4096)))
  }
  const media = nativeMediaBlock(attachment.mediaType, fileName, data)
  return media ? [media, record] : [record]
}

/** The block a model reads natively, or null for a file it only reaches by path. */
function nativeMediaBlock(
  mediaType: string,
  fileName: string,
  data: Uint8Array
): UserContentBlock | null {
  const sniffed = sniffModelMediaType(data)
  if (sniffed) {
    return {
      type: sniffed.kind,
      source: { type: 'binary', data, mediaType: sniffed.mediaType }
    }
  }
  if (isPdf(mediaType, data)) {
    return {
      type: 'document',
      source: { data, mediaType: 'application/pdf', fileName }
    }
  }
  return null
}

function isPdf(mediaType: string, data: Uint8Array): boolean {
  if (mediaType.split(';')[0]?.trim() === 'application/pdf')
    return true
  return new TextDecoder().decode(data.subarray(0, 5)) === '%PDF-'
}

function missing(ref: string): UserContentBlock {
  return { type: 'text', text: `[attachment ${ref} is not available]` }
}

/**
 * Writes an attachment to the host under the host user's home,
 * `~/.demi/attachments/<conversation id>/`: outside every working directory,
 * and kept, so the path a transcript names stays readable. A name already
 * there gets a numeric suffix before the extension, so nothing is
 * overwritten. Returns the absolute path.
 */
export async function writeAttachmentToHost(
  host: Host,
  conversationId: string,
  fileName: string,
  data: Uint8Array
): Promise<string> {
  const home = host.identity.homeDir.replace(/\/+$/, '')
  const directory = `${home}/${ATTACHMENTS_DIR}/${conversationId}`
  let candidate = fileName
  for (let index = 2; await host.fs.exists(`${directory}/${candidate}`); index += 1) {
    candidate = numberedFileName(fileName, index)
  }
  const path = `${directory}/${candidate}`
  await host.fs.writeFile(path, data, { createParents: true })
  return path
}

function numberedFileName(fileName: string, index: number): string {
  const dot = fileName.lastIndexOf('.')
  if (dot <= 0)
    return `${fileName}-${index}`
  return `${fileName.slice(0, dot)}-${index}${fileName.slice(dot)}`
}
