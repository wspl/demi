import type { Block } from '@demicodes/core'
import type { TranscriptPatch } from './frames'

/**
 * Applies journal-produced transcript patches to a block list, returning a new
 * array. Touched blocks are replaced (never mutated in place), so callers can
 * safely share block objects between snapshots.
 */
export function applyTranscriptPatches(blocks: Block[], patches: TranscriptPatch[]): Block[] {
  let next = [...blocks]
  for (const patch of patches) {
    switch (patch.op) {
      case 'replace':
        next = [...patch.value]
        break
      case 'replace_block':
        next[patch.path[1]] = patch.value
        break
      case 'add':
        next.splice(patch.path[1], 0, patch.value)
        break
      case 'remove':
        next.splice(patch.path[1], 1)
        break
      case 'append_text': {
        const index = patch.path[1]
        const block = next[index]
        if (block && (block.type === 'text' || block.type === 'thinking')) {
          next[index] = { ...block, text: block.text + patch.delta }
        }
        break
      }
    }
  }
  return next
}

export function cloneBlocks(blocks: Block[]): Block[] {
  return structuredClone(blocks)
}
