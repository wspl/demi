import { inject, provide, type InjectionKey } from 'vue'
import { decodedTarget } from '../../markdown/filePath'
import type { MessageEditContent } from '../message-editing'
import type { MediaSource } from '../media-source'
import { attachmentProgress, decodeRemoteReference, type ComposerAttachment } from '../message-input/attachments'

/**
 * A file as its capsule in a message shows it (`product.md` § Attachments):
 * a picture or its kind's icon, then its name.
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
  /** An upload on its way, 0 to 1, or one that failed and can be tried again. */
  upload?: { phase: 'uploading'; progress: number } | { phase: 'failed' }
}

/** What the capsules in one editor read: their files, and whether a capsule can be removed or retried. */
export interface CapsuleContext {
  capsule(id: string): MessageCapsule | undefined
  editable(): boolean
  retry(id: string): void
}

const capsuleContextKey: InjectionKey<CapsuleContext> = Symbol('message-capsules')

export function provideCapsules(context: CapsuleContext): void {
  provide(capsuleContextKey, context)
}

export function useCapsules(): CapsuleContext {
  const context = inject(capsuleContextKey)
  if (!context) {
    throw new Error('A capsule renders inside a message editor')
  }
  return context
}

/** A file on its way to a message, as its capsule shows it: an upload, or a file on another device. */
export function composerCapsule(item: ComposerAttachment): MessageCapsule {
  if (item.kind === 'reference') {
    return { id: item.id, name: item.name, host: item.host, path: item.path }
  }
  return {
    id: item.id,
    name: item.name,
    image: item.src,
    snippet: item.snippet,
    upload: item.phase === 'uploading'
      ? { phase: 'uploading', progress: attachmentProgress(item) }
      : item.phase === 'failed' ? { phase: 'failed' } : undefined,
  }
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
