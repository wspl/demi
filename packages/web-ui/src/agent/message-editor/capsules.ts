import { inject, provide, type InjectionKey } from 'vue'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { z } from 'zod'
import { baseName, jsonOrNull } from '@demicodes/utils'
import type { MediaSource as ContentMediaSource } from '@demicodes/protocol'
import { decodedTarget } from '../../markdown/filePath'
import type { MessageEditContent } from '../message-editing'
import type { MediaSource } from '../media-source'
import { attachmentProgress, decodeRemoteReference, type ComposerAttachment } from '../message-input/attachments'
import type { FileReference } from '../message-input/copied-message'
import { splitMessageContent } from '../message-input/message-content'

/**
 * A file as its capsule in a message shows it, and as the message carries it
 * (`product.md` § Attachments). The capsule's node keeps these, so the
 * document alone says which files a message has, in what order, and what each
 * one is.
 */
export interface MessageCapsule {
  id: string
  name: string
  /** A picture's address, or the media block it loads from. */
  image?: string | MediaSource
  /** A video's media block, whose first frame its preview on hover shows. */
  video?: MediaSource
  /** The opening of a text file, shown when the capsule is pointed at. */
  snippet?: string
  /** The device a file on another device is on. */
  host?: string
  /** Where the file is: on the Host, or on `host`. */
  path?: string
  /**
   * The file as a sent message holds it, which a copy of the message carries
   * so that a paste brings the file back without its bytes; none for a file
   * still on its way.
   */
  reference?: FileReference
}

/** The file a node of a message's document stands for, when it is a capsule. */
export function capsuleOf(node: ProseMirrorNode): MessageCapsule | null {
  return node.type.name === 'attachment' ? node.attrs['capsule'] ?? null : null
}

/** An image file's picture, from its bytes in the user's blobs; none for a file of another kind. */
export function blobImage(mediaType: string, sha256: string): ContentMediaSource | undefined {
  return mediaType.startsWith('image/') ? { type: 'ref', ref: sha256, mediaType } : undefined
}

/** A video file's bytes in the user's blobs; none for a file of another kind. */
function blobVideo(mediaType: string, sha256: string): ContentMediaSource | undefined {
  return mediaType.startsWith('video/') ? { type: 'ref', ref: sha256, mediaType } : undefined
}

/** What is happening to a file the composer is still carrying to the Host. */
export type MessageTransfer =
  | { phase: 'uploading'; progress: number }
  /** A pasted message's stored file being attached, which has no progress to tell. */
  | { phase: 'attaching' }
  | { phase: 'failed' }

/**
 * The files one composer is carrying, under their capsules' ids: how far each
 * is on its way, and whether the composer can carry a capsule at all, which
 * is what a paste asks. A capsule with no transfer is a file that needs none:
 * a restored draft, or a message already sent.
 */
export interface TransferContext {
  transfer(id: string): MessageTransfer | undefined
  carries(id: string): boolean
  retry(id: string): void
  /**
   * The file as the composer knows it now, which the capsule the document
   * saved may predate: an upload's answer brings a text file's opening.
   */
  current(id: string): MessageCapsule | undefined
}

/**
 * A capsule as it travels on the clipboard. A picture is its address there,
 * never the bytes of a sent message's media, which copy as the file's name.
 */
const capsuleSchema = z.object({
  id: z.string().min(1),
  name: z.string(),
  image: z.string().optional(),
  snippet: z.string().optional(),
  host: z.string().optional(),
  path: z.string().optional(),
})

/** The capsule a pasted node carries, or nothing when it comes from outside the app. */
export function readCapsule(value: string | null): MessageCapsule | null {
  if (!value) {
    return null
  }
  const read = capsuleSchema.safeParse(jsonOrNull(value))
  return read.success ? read.data : null
}

const transferContextKey: InjectionKey<TransferContext> = Symbol('message-transfers')

export function provideTransfers(context: TransferContext): void {
  provide(transferContextKey, context)
}

/** The transfers beside this editor's capsules; none in a conversation, where every file has arrived. */
export function useTransfers(): TransferContext | null {
  return inject(transferContextKey, null)
}

/** A file on its way to a message, as its capsule shows it: an upload, or a file on another device. */
export function composerCapsule(item: ComposerAttachment): MessageCapsule {
  if (item.kind === 'reference') {
    return { id: item.id, name: item.name, host: item.host, path: item.path }
  }
  return {
    id: item.id,
    name: item.name,
    ...(item.src ? { image: item.src } : {}),
    ...(item.snippet ? { snippet: item.snippet } : {}),
  }
}

/** How far a file has come, while it is still on its way; nothing once it has arrived. */
export function composerTransfer(item: ComposerAttachment): MessageTransfer | undefined {
  if (item.kind !== 'file') {
    return undefined
  }
  if (item.phase === 'uploading') {
    return { phase: 'uploading', progress: attachmentProgress(item) }
  }
  if (item.phase === 'attaching') {
    return { phase: 'attaching' }
  }
  return item.phase === 'failed' ? { phase: 'failed' } : undefined
}

/**
 * A file a message carried, as its capsule shows it: its attachment record
 * with the picture of the media block before it, a media block alone, a
 * reference to a file on another device, or a file an edit uploaded.
 */
export function contentCapsule(id: string, blocks: readonly MessageEditContent[]): MessageCapsule {
  let capsule: MessageCapsule = { id, name: 'file' }
  for (const block of blocks) {
    if (block.type === 'reference') {
      const { host, path, name, deviceId } = decodeRemoteReference(block.reference)
      capsule = { ...capsule, name, host, path, ...(deviceId ? { reference: { type: 'device', deviceId } } : {}) }
    } else if (block.type === 'remote_file') {
      capsule = { ...capsule, name: baseName(block.path), host: block.host, path: block.path, reference: { type: 'device', deviceId: block.deviceId } }
    } else if (block.type === 'attachment') {
      // The media block before the record, fitted for the model, shows the
      // picture; without one, as for a model that reads no images, the
      // original does.
      capsule = {
        ...capsule,
        name: block.name,
        path: block.path,
        snippet: block.snippet,
        image: capsule.image ?? blobImage(block.mediaType, block.sha256),
        video: capsule.video ?? blobVideo(block.mediaType, block.sha256),
        reference: { type: 'stored', sha256: block.sha256, mediaType: block.mediaType, size: block.sizeBytes },
      }
    } else if (block.type === 'document') {
      capsule = {
        ...capsule,
        name: block.source.fileName,
        reference: { type: 'stored', sha256: block.source.ref, mediaType: block.source.mediaType },
      }
    } else if (block.type === 'image' || block.type === 'video') {
      const { source } = block
      capsule = {
        ...capsule,
        name: mediaName(block),
        image: block.type === 'image' ? source : undefined,
        video: block.type === 'video' ? source : undefined,
        ...(source.type === 'ref' ? { reference: { type: 'stored', sha256: source.ref, mediaType: source.mediaType } } : {}),
      }
    } else if (block.type === 'upload') {
      capsule = {
        ...capsule,
        name: block.fileName,
        snippet: block.snippet,
        image: blobImage(block.mediaType, block.sha256),
        video: blobVideo(block.mediaType, block.sha256),
        reference: { type: 'stored', sha256: block.sha256, mediaType: block.mediaType },
      }
    }
  }
  return capsule
}

/** A message the session holds as its editor shows it: its Markdown, and a capsule for each file in the order of their marks. */
export function contentMessage(content: readonly MessageEditContent[]): { markdown: string; capsules: MessageCapsule[] } {
  const { markdown, files } = splitMessageContent(content)
  return { markdown, capsules: files.map((blocks, index) => contentCapsule(String(index), blocks)) }
}

/**
 * An image a message names, as the capsule the user's message shows it by
 * (`file-previews.md` § Files named in messages): named as the user alt-texted
 * it, else by its file or address, its picture the one loaded from `src`, and
 * a Host image at its path, which a click opens. The message's Markdown is not
 * touched; only how it shows changes.
 */
export function imageCapsule(image: { url: string } | { path: string }, src: string, alt: string): MessageCapsule {
  if ('path' in image) {
    return { id: `image:${image.path}`, name: alt || baseName(image.path), image: src, path: image.path }
  }
  const leaf = image.url.startsWith('data:') ? undefined : image.url.split(/[?#]/)[0]?.split('/').pop()
  return { id: `image:${image.url}`, name: alt || (leaf ? decodedTarget(leaf) : 'image'), image: src }
}

/** A media block's name, when no record names it: the leaf of its address, or its kind. */
function mediaName(block: Extract<MessageEditContent, { type: 'image' | 'video' }>): string {
  const { source } = block
  if (source.type === 'url') {
    const leaf = source.url.split(/[?#]/)[0]?.split('/').pop()
    if (leaf) {
      return decodedTarget(leaf)
    }
  }
  return block.type
}
