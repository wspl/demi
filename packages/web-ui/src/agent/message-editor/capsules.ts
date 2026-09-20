import { inject, provide, type InjectionKey } from 'vue'
import { z } from 'zod'
import { decodedTarget } from '../../markdown/filePath'
import type { MessageEditContent } from '../message-editing'
import type { MediaSource } from '../media-source'
import { attachmentProgress, decodeRemoteReference, type ComposerAttachment } from '../message-input/attachments'

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
  /** The opening of a text file, shown when the capsule is pointed at. */
  snippet?: string
  /** The device a file on another device is on. */
  host?: string
  /** Where the file is: on the Host, or on `host`. */
  path?: string
}

/** What is happening to a file the composer is still carrying to the Host. */
export type MessageTransfer =
  | { phase: 'uploading'; progress: number }
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

function jsonOrNull(value: string): unknown {
  try {
    return JSON.parse(value)
  } catch {
    return null
  }
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
  return item.phase === 'failed' ? { phase: 'failed' } : undefined
}

/**
 * A file a message carried, as its capsule shows it: its attachment record
 * with the picture of the media block before it, a media block alone, or a
 * reference to a file on another device.
 */
export function contentCapsule(id: string, blocks: readonly MessageEditContent[]): MessageCapsule {
  let capsule: MessageCapsule = { id, name: 'file' }
  for (const block of blocks) {
    if (block.type === 'reference') {
      const { host, path, name } = decodeRemoteReference(block.reference)
      capsule = { ...capsule, name, host, path }
    } else if (block.type === 'attachment') {
      capsule = { ...capsule, name: block.name, path: block.path, snippet: block.snippet }
    } else if (block.type === 'document') {
      capsule = { ...capsule, name: block.source.fileName ?? capsule.name }
    } else if (block.type === 'image' || block.type === 'video') {
      capsule = { ...capsule, name: mediaName(block), image: block.type === 'image' ? block.source : undefined }
    }
  }
  return capsule
}

/** A media block's name, when no record names it: its file's, or its kind. */
function mediaName(block: Extract<MessageEditContent, { type: 'image' | 'video' }>): string {
  const { source } = block
  if (source.type === 'ref' && source.fileName) {
    return source.fileName
  }
  if (source.type === 'url') {
    const leaf = source.url.split(/[?#]/)[0]?.split('/').pop()
    if (leaf) {
      return decodedTarget(leaf)
    }
  }
  return block.type
}
