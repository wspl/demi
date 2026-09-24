import type { Block, TranscriptPatch } from '@demicodes/protocol'

/**
 * The one transcript patch applier (`contracts.md` § Generated TypeScript):
 * `blocks` with `patches` applied in order, as a new array. A block a patch
 * touches is replaced, never changed in place, so snapshots can share
 * blocks.
 */
export function applyTranscriptPatches(blocks: readonly Block[], patches: readonly TranscriptPatch[]): Block[] {
  let next = [...blocks]
  for (const patch of patches) {
    switch (patch.op) {
      case 'replace':
        next = [...patch.value]
        break
      case 'replace_block':
        next[patch.index] = patch.value
        break
      case 'add':
        next.splice(patch.index, 0, patch.value)
        break
      case 'remove':
        next.splice(patch.index, 1)
        break
      case 'append_text': {
        const block = next[patch.index]
        if (block?.type === 'text' || block?.type === 'thinking') {
          next[patch.index] = { ...block, text: block.text + patch.delta }
        }
        break
      }
    }
  }
  return next
}
