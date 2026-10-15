import type { Versions } from '@demicodes/protocol'
import type { MessageListBlock } from './pending-steers'

/**
 * The versions a switcher offers (`message-editing.md` § Versions): where
 * the shown one stands, `index` of `count` from 1, whether switching waits
 * for the agent, and how to choose another.
 */
export interface VersionChoice extends Versions {
  disabled: boolean
  choose: (to: number) => void
}

/** The versions at a block's place on the path, by the block whose place it is. */
export interface VersionPlace extends Versions {
  blockId: string
}

/** A block's versions, when its place holds others. */
function versionsOf(block: MessageListBlock): Versions | undefined {
  return 'versions' in block ? block.versions : undefined
}

/**
 * Where the transcript shows its versions, by the row that shows them: an
 * edited message under itself, and a regenerated answer in its footer, the
 * row in `footerIds` that ends its turn, from the first block after its
 * message whose place holds versions. The answer's first block may be one
 * the page does not show, such as a context block, so `blocks` are all the
 * blocks the page holds.
 */
export function versionPlaces(
  blocks: readonly MessageListBlock[],
  footerIds: ReadonlySet<string>,
): Map<string, VersionPlace> {
  const places = new Map<string, VersionPlace>()
  let answer: VersionPlace | null = null
  for (const block of blocks) {
    const versions = versionsOf(block)
    if (block.type === 'user') {
      answer = null
      if (versions) {
        places.set(block.id, { blockId: block.id, ...versions })
      }
      continue
    }
    if (versions && !answer) {
      answer = { blockId: block.id, ...versions }
    }
    if (answer && footerIds.has(block.id)) {
      places.set(block.id, answer)
    }
  }
  return places
}
