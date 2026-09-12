import type { Block } from '@demicodes/core'
import type { TranscriptPatch } from '../protocol/frames'

/**
 * Applies journal-produced transcript patches to a block list, returning a new
 * array. Touched blocks are replaced (never mutated in place), so callers can
 * safely share block objects between snapshots.
 */
export function applyTranscriptPatches<B extends Block<unknown, unknown>>(
  blocks: B[],
  patches: TranscriptPatch<B>[]
): B[] {
  let next = [...blocks]
  for (const patch of patches) {
    switch (patch.op) {
      case 'replace':
        next = [...patch.value]
        break
      case 'replace_block':
        requireBlock(next, patch.path[1])
        next[patch.path[1]] = patch.value
        break
      case 'add':
        if (!Number.isInteger(patch.path[1]) || patch.path[1] < 0 || patch.path[1] > next.length)
          throw new Error('Invalid transcript insertion index')
        next.splice(patch.path[1], 0, patch.value)
        break
      case 'remove':
        requireBlock(next, patch.path[1])
        next.splice(patch.path[1], 1)
        break
      case 'append_text': {
        const index = patch.path[1]
        const block = requireBlock(next, index)
        if (block.type !== 'text' && block.type !== 'thinking')
          throw new Error('Invalid transcript text append target')
        next[index] = { ...block, text: block.text + patch.delta }
        break
      }
    }
  }
  return next
}

export function cloneBlocks(blocks: Block[]): Block[] {
  return structuredClone(blocks)
}

function requireBlock<B>(blocks: B[], index: number): B {
  if (!Number.isInteger(index) || index < 0 || index >= blocks.length)
    throw new Error('Invalid transcript block index')
  return blocks[index]
}
