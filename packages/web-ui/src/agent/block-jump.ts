import { inject, provide, type InjectionKey } from 'vue'

/**
 * Brings a block of the transcript a list shows into view and highlights it
 * for a moment, as a chat app jumps to a quoted message.
 */
export type BlockJump = (blockId: string) => void

const blockJumpKey: InjectionKey<BlockJump> = Symbol('block-jump')

/** Gives the rows below the jump to a block of their list. */
export function provideBlockJump(jump: BlockJump): void {
  provide(blockJumpKey, jump)
}

/** The jump to a block of the list a row is in; none for a block shown on its own. */
export function useBlockJump(): BlockJump | undefined {
  return inject(blockJumpKey, undefined)
}
