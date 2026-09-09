import { onMounted, onUnmounted } from 'vue'
import { matchesShortcut } from '../ui/shortcut'

/** Bind configured application actions for the lifetime of the owning surface. */
export function useAppShortcuts(
  enabled: () => boolean,
  bindings: () => readonly {
    id: string
    keys: string
  }[],
  actions: Record<string, () => void>,
): void {
  function handle(event: KeyboardEvent): void {
    if (!enabled()) {
      return
    }
    const binding = bindings().find((entry) => matchesShortcut(event, entry.keys))
    const action = binding && actions[binding.id]
    if (!action) {
      return
    }
    event.preventDefault()
    action()
  }
  onMounted(() => window.addEventListener('keydown', handle))
  onUnmounted(() => window.removeEventListener('keydown', handle))
}
