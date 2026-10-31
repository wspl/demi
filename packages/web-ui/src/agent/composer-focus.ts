import { inject, onBeforeUnmount, provide, type InjectionKey } from 'vue'

/**
 * The way to the composer of a conversation's surface, for what is done
 * elsewhere in it and leaves the next step to the composer, as a comment
 * saved to the draft leaves its sending: the surface (`ChatSession`) holds
 * it, its composer (`SessionComposer`) takes it while it shows, and the
 * transcript's comments (`ReplyComments`) call it.
 */
export interface ComposerFocus {
  /** Puts the focus in the composer, its caret at the draft's end; nothing while none shows. */
  focus(): void
}

interface ComposerFocusHolder extends ComposerFocus {
  take(focus: () => void): () => void
}

const composerFocusKey: InjectionKey<ComposerFocusHolder> = Symbol('composer-focus')

/** The surface's way to its composer, which its composer takes as it mounts. */
export function provideComposerFocus(): ComposerFocus {
  let current: (() => void) | null = null
  const holder: ComposerFocusHolder = {
    focus: () => current?.(),
    take(focus) {
      current = focus
      return () => {
        if (current === focus) {
          current = null
        }
      }
    },
  }
  provide(composerFocusKey, holder)
  return holder
}

/** The composer takes the surface's way to it while it is mounted; none outside a surface. */
export function takeComposerFocus(focus: () => void): void {
  const release = inject(composerFocusKey, null)?.take(focus)
  if (release) {
    onBeforeUnmount(release)
  }
}

/** The surface's way to its composer; doing nothing outside a surface. */
export function useComposerFocus(): ComposerFocus {
  return inject(composerFocusKey, null) ?? { focus: () => {} }
}
