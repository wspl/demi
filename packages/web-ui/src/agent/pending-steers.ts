import type { Block, ImageSource, UserContentBlock } from '@demicodes/core'
import type { PendingSteerMessage } from './types'

export interface PendingSteerRenderBlock {
  type: 'pending_steer'
  id: string
  pendingSteerId: string
  content: UserContentBlock[]
}

export type MessageListBlock = Block | PendingSteerRenderBlock

export function createPendingSteerMessage(
  id: string,
  content: UserContentBlock[],
  blocks: readonly Block[],
): PendingSteerMessage {
  return {
    id,
    content: cloneUserContent(content),
    baselineSteerBlockIds: blocks.flatMap((block) => (block.type === 'steer' ? [block.id] : [])),
  }
}

export function reconcilePendingSteers(
  blocks: readonly Block[],
  pendingSteers: readonly PendingSteerMessage[],
): PendingSteerMessage[] {
  if (pendingSteers.length === 0) return pendingSteers as PendingSteerMessage[]

  const steerBlocks = blocks.filter((block): block is Extract<Block, { type: 'steer' }> => block.type === 'steer')
  const consumedBlockIds = new Set<string>()
  const remaining: PendingSteerMessage[] = []

  for (const pending of pendingSteers) {
    const candidates = steerBlocks.filter(
      (block) => !consumedBlockIds.has(block.id) && !pending.baselineSteerBlockIds.includes(block.id),
    )
    const exactMatch = candidates.find((block) => contentKey(block.content) === contentKey(pending.content))
    const materializedBlock = exactMatch ?? candidates[0]

    if (materializedBlock) {
      consumedBlockIds.add(materializedBlock.id)
    } else {
      remaining.push(pending)
    }
  }

  return remaining.length === pendingSteers.length ? (pendingSteers as PendingSteerMessage[]) : remaining
}

export function pendingSteersToRenderBlocks(pendingSteers: readonly PendingSteerMessage[]): PendingSteerRenderBlock[] {
  return pendingSteers.map((pending) => ({
    type: 'pending_steer',
    id: `pending-steer:${pending.id}`,
    pendingSteerId: pending.id,
    content: pending.content,
  }))
}

function contentKey(content: readonly UserContentBlock[]): string {
  return JSON.stringify(content.map(normalizeContentBlock))
}

function normalizeContentBlock(block: UserContentBlock): unknown {
  switch (block.type) {
    case 'text':
      return { type: block.type, text: block.text }
    case 'reference':
      return { type: block.type, reference: block.reference }
    case 'image':
    case 'video':
      return {
        type: block.type,
        source:
          block.source.type === 'url'
            ? block.source
            : { type: 'binary', mediaType: block.source.mediaType, data: normalizeBinary(block.source.data) },
      }
    case 'document':
      return {
        type: block.type,
        source: {
          fileName: block.source.fileName,
          mediaType: block.source.mediaType,
          data: normalizeBinary(block.source.data),
        },
      }
  }
}

function normalizeBinary(data: Uint8Array): number[] {
  return Array.from(data)
}

function cloneUserContent(content: readonly UserContentBlock[]): UserContentBlock[] {
  return content.map((block) => {
    switch (block.type) {
      case 'text':
        return { type: 'text', text: block.text }
      case 'reference':
        return { type: 'reference', reference: block.reference }
      case 'image':
        return {
          type: 'image',
          source: cloneMediaSource(block.source),
        }
      case 'video':
        return {
          type: 'video',
          source: cloneMediaSource(block.source),
        }
      case 'document':
        return {
          type: 'document',
          source: {
            fileName: block.source.fileName,
            mediaType: block.source.mediaType,
            data: new Uint8Array(Array.from(block.source.data)),
          },
        }
    }
  })
}

function cloneMediaSource(source: ImageSource): ImageSource {
  return source.type === 'url'
    ? { type: 'url', url: source.url }
    : {
        type: 'binary',
        mediaType: source.mediaType,
        data: new Uint8Array(Array.from(source.data)),
      }
}
