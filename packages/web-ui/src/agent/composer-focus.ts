import { inject, onBeforeUnmount, provide, type InjectionKey } from 'vue'

/**
 * The way to the composer of a conversation's surface, for what is done
 * elsewhere in it and leaves the next step to the composer, as a comment
 * saved to the draft leaves its sending, or a suggested reply's Edit its
 * words: the surface (`ChatSession`) holds it, its composer
 * (`SessionComposer`) takes it while it shows, and the transcript's comments
 * (`ReplyComments`) and suggested replies call it.
 */
export interface ComposerFocus {
  /**
   * Puts the focus in the composer, its caret at the draft's end, after adding
   * `words` there, as a line of their own after a draft already there;
   * nothing while none shows.
   */
  focus(words?: string): void
}

interface ComposerFocusHolder extends ComposerFocus {
  take(focus: (words?: string) => void): () => void
}

const composerFocusKey: InjectionKey<ComposerFocusHolder> = Symbol('composer-focus')

/** The surface's way to its composer, which its composer takes as it mounts. */
export function provideComposerFocus(): ComposerFocus {
  let current: ((words?: string) => void) | null = null
  const holder: ComposerFocusHolder = {
    focus: (words) => current?.(words),
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
export function takeComposerFocus(focus: (words?: string) => void): void {
  const release = inject(composerFocusKey, null)?.take(focus)
  if (release) {
    onBeforeUnmount(release)
  }
}

/** The surface's way to its composer; doing nothing outside a surface. */
export function useComposerFocus(): ComposerFocus {
  return inject(composerFocusKey, null) ?? { focus: () => {} }
}
