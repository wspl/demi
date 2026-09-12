import { base64ToBytes, bytesToBase64 } from '@demicodes/utils'
import type { Block, ToolResultContentBlock, UserContentBlock } from '@demicodes/core'
import type { StoredBlock, StoredToolContent, StoredUserContent } from './media-contracts'

import type { BlobStore } from './blob'
export type { BlobStore } from './blob'

/** Moves inline media into blob storage; URL sources remain URLs. */
export async function externalizeBlockMedia(block: Block, blobs: BlobStore): Promise<StoredBlock> {
  if (block.type === 'user') {
    return {
      ...block,
      content: await Promise.all(block.content.map((item) => externalizeUserContent(item, blobs))),
      resolvedContent: block.resolvedContent
        ? await Promise.all(
            block.resolvedContent.map((item) => externalizeUserContent(item, blobs)),
          )
        : undefined,
    }
  }
  if (block.type === 'steer') {
    return {
      ...block,
      content: await Promise.all(block.content.map((item) => externalizeUserContent(item, blobs))),
    }
  }
  if (block.type === 'tool_call') {
    return {
      ...block,
      output: await Promise.all(block.output.map((item) => externalizeToolResult(item, blobs))),
      streamingOutput: await Promise.all(
        block.streamingOutput.map((item) => externalizeToolResult(item, blobs)),
      ),
    }
  }
  return block
}

/** Rehydrates validated stored blocks. Only absent blobs become text placeholders. */
export async function rehydrateBlockMedia(block: StoredBlock, blobs: BlobStore): Promise<Block> {
  if (block.type === 'user') {
    return {
      ...block,
      content: await Promise.all(block.content.map((item) => rehydrateUserContent(item, blobs))),
      resolvedContent: block.resolvedContent
        ? await Promise.all(block.resolvedContent.map((item) => rehydrateUserContent(item, blobs)))
        : undefined,
    }
  }
  if (block.type === 'steer') {
    return {
      ...block,
      content: await Promise.all(block.content.map((item) => rehydrateUserContent(item, blobs))),
    }
  }
  if (block.type === 'tool_call') {
    return {
      ...block,
      output: await Promise.all(block.output.map((item) => rehydrateToolResult(item, blobs))),
      streamingOutput: await Promise.all(
        block.streamingOutput.map((item) => rehydrateToolResult(item, blobs)),
      ),
    }
  }
  return block
}

async function externalizeUserContent(
  item: UserContentBlock,
  blobs: BlobStore,
): Promise<StoredUserContent> {
  if (item.type === 'document') {
    return {
      ...item,
      source: {
        type: 'ref',
        ref: await blobs.put(item.source.data),
        mediaType: item.source.mediaType,
        fileName: item.source.fileName,
      },
    }
  }
  if (item.type === 'image' || item.type === 'video') {
    if (item.source.type === 'url') return { ...item, source: item.source }
    return {
      ...item,
      source: {
        type: 'ref',
        ref: await blobs.put(item.source.data),
        mediaType: item.source.mediaType,
      },
    }
  }
  return item
}

async function rehydrateUserContent(
  item: StoredUserContent,
  blobs: BlobStore,
): Promise<UserContentBlock> {
  if (item.type === 'document') {
    const data = await blobs.get(item.source.ref)
    return data === null
      ? missingMediaPlaceholder(item.type, item.source.ref)
      : {
          ...item,
          source: { data, mediaType: item.source.mediaType, fileName: item.source.fileName },
        }
  }
  if (item.type === 'image' || item.type === 'video') {
    if (item.source.type === 'url') return { ...item, source: item.source }
    const data = await blobs.get(item.source.ref)
    return data === null
      ? missingMediaPlaceholder(item.type, item.source.ref)
      : { ...item, source: { type: 'binary', data, mediaType: item.source.mediaType } }
  }
  return item
}

async function externalizeToolResult(
  item: ToolResultContentBlock,
  blobs: BlobStore,
): Promise<StoredToolContent> {
  if (item.type === 'text') return item
  return {
    ...item,
    source: {
      ref: await blobs.put(base64ToBytes(item.source.data)),
      mediaType: item.source.mediaType,
    },
  }
}

async function rehydrateToolResult(
  item: StoredToolContent,
  blobs: BlobStore,
): Promise<ToolResultContentBlock> {
  if (item.type === 'text') return item
  const data = await blobs.get(item.source.ref)
  return data === null
    ? missingMediaPlaceholder(item.type, item.source.ref)
    : { ...item, source: { mediaType: item.source.mediaType, data: bytesToBase64(data) } }
}

function missingMediaPlaceholder(
  kind: 'image' | 'video' | 'document',
  ref: string,
): { type: 'text'; text: string } {
  return { type: 'text', text: `[missing ${kind} blob ${ref}]` }
}
