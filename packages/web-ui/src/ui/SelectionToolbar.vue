<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { autoUpdate, flip, offset, shift, useFloating } from '@floating-ui/vue'
import { stackAbove } from '../overlay/stacking'
import { usePhone } from './phone'
import { selectedLinesBounds, selectionTools, shownContextMenu, type SelectionTools } from './context-menu'
import SelectionToolRow from './SelectionToolRow.vue'

/**
 * The text selection's toolbar, which the app mounts once at its root.
 *
 * While the selection holds one object's own text and the object offers tools
 * for it, as a message does (`selectionTools`), a row of its tools stands by it:
 *
 * - On the desktop it shows the tools that act in place, such as Comment. The
 *   right-click menu holds Copy and the rest.
 * - On a phone it shows every tool the object offers, such as Send to
 *   Conversation. Copy is the system's callout's.
 *
 * Placement and timing:
 *
 * - It is centered on the bounds of the selected text's lines. On the desktop it
 *   stands above them, once the mouse that drags the selection is released. On a
 *   phone it stands under them, clear of the system's callout above.
 * - It shows once the selection is made, already where it stands.
 * - It steps aside while a context menu is open, the one control for the
 *   selection then, and comes back as the menu closes on the selection still
 *   there.
 * - While a tool's submenu is open (SelectionToolRow), the toolbar keeps the
 *   selection it opened for, though the focus moving into the submenu's search
 *   field ends the selection on the page.
 */

const phone = usePhone()
const tools = shallowRef<SelectionTools | null>(null)
/** A mouse holds the selection it drags: the toolbar waits for its release. */
const dragging = ref(false)
/** A tool's submenu is open, which holds the toolbar where it is. */
const holding = ref(false)

function read(): void {
  if (holding.value)
    return
  tools.value = selectionTools(phone.value)
}

const listeners = new AbortController()
onMounted(() => {
  const { signal } = listeners
  document.addEventListener('selectionchange', read, { signal })
  document.addEventListener('mousedown', (event) => {
    if (event.button === 0 && !floating.value?.contains(event.target as Node))
      dragging.value = true
  }, { capture: true, signal })
  document.addEventListener('mouseup', () => {
    dragging.value = false
    read()
  }, { capture: true, signal })
})
onBeforeUnmount(() => listeners.abort())

/**
 * A selection whose text the page renders again, as a reply that streams does,
 * collapses without a `selectionchange` in Chrome. The page's change of the selected
 * text is read as the selection's, so the toolbar never stands for text that went.
 */
watch(tools, (now, _before, onCleanup) => {
  // The object the text is in, whose part holding the text may itself be replaced.
  if (!now)
    return
  const observer = new MutationObserver(read)
  observer.observe(now.object, { childList: true, subtree: true, characterData: true })
  onCleanup(() => observer.disconnect())
})

const shown = computed(() => tools.value !== null && !dragging.value && shownContextMenu.value === null)

const reference = computed(() => {
  const now = tools.value
  if (!now)
    return null
  return {
    contextElement: now.object,
    // The selected lines' bounds, whose top the desktop's toolbar stands above
    // and whose bottom a phone's stands under.
    getBoundingClientRect: () => selectedLinesBounds(now.range) ?? now.range.getBoundingClientRect(),
  }
})

const floating = ref<HTMLElement>()
const { floatingStyles, isPositioned } = useFloating(reference, floating, {
  // Drawn only once placed: before its first placement it stands at the page's corner.
  open: shown,
  placement: computed(() => (phone.value ? 'bottom' : 'top')),
  strategy: 'fixed',
  // A phone's selection has its handles below its last line, which the toolbar clears.
  middleware: computed(() => [offset(phone.value ? 28 : 8), flip({ padding: 8 }), shift({ padding: 8 })]),
  whileElementsMounted: autoUpdate,
})
const z = stackAbove()

function hold(now: boolean): void {
  holding.value = now
  if (!now)
    read()
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="shown && tools"
      ref="floating"
      data-selection-toolbar
      :style="{ ...floatingStyles, zIndex: z(), visibility: isPositioned ? undefined : 'hidden' }"
    >
      <SelectionToolRow :entries="tools.entries" @holding="hold" />
    </div>
  </Teleport>
</template>
