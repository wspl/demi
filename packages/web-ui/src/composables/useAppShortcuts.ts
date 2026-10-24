import { isFocusedElementEditable, useEventListener } from '@vueuse/core'
import { holdsCommandModifier, matchesShortcut } from '../ui/shortcut'

/**
 * Binds configured application actions for the lifetime of the owning
 * surface or effect scope. The focused control has the keys first, as in
 * every desktop app: a key it used, such as ⌘B for bold in the composer, runs
 * no shortcut, and in a field a shortcut without ⌘ or ⌃ (Ctrl on a PC) never runs, since
 * those keys type. An action that answers `false` declined the keys, which then
 * do what they do without it, as ⌘F finds in the browser's page.
 */
export function useAppShortcuts(
  enabled: () => boolean,
  bindings: () => readonly {
    id: string
    keys: string
  }[],
  actions: Record<string, () => void | boolean>,
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
    if (isFocusedElementEditable() && !holdsCommandModifier(binding.keys)) {
      return
    }
    if (action() !== false)
      event.preventDefault()
  }
  useEventListener(window, 'keydown', handle)
}
