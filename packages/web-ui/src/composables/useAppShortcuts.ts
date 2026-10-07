import { isFocusedElementEditable, useEventListener } from '@vueuse/core'
import { hasCommandModifier } from '../settings/shortcuts'
import { matchesShortcut } from '../ui/shortcut'

/**
 * Binds configured application actions for the lifetime of the owning
 * surface or effect scope. The focused control has the keys first, as in
 * every desktop app: a key it used, such as ⌘B for bold in the composer, runs
 * no shortcut, and in a field a shortcut without ⌘ or ⌃ never runs, since
 * those keys type.
 */
export function useAppShortcuts(
  enabled: () => boolean,
  bindings: () => readonly {
    id: string
    keys: string
  }[],
  actions: Record<string, () => void>,
): void {
  function handle(event: KeyboardEvent): void {
    if (!enabled() || event.defaultPrevented) {
      return
    }
    const binding = bindings().find((entry) => matchesShortcut(event, entry.keys))
    const action = binding && actions[binding.id]
    if (!action) {
      return
    }
    if (isFocusedElementEditable() && !hasCommandModifier(binding.keys)) {
      return
    }
    event.preventDefault()
    action()
  }
  useEventListener(window, 'keydown', handle)
}
