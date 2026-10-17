<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, ref, shallowRef } from 'vue'
import { autoUpdate, flip, offset, shift, useFloating } from '@floating-ui/vue'
import { Check } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { useCopy } from '../composables/useCopy'
import { stackAbove } from '../overlay/stacking'
import Button from './Button.vue'
import Popover from './Popover.vue'
import { ICON_PX } from './icon-metrics'
import { menuRootKey } from './menu-context'
import { usePhone } from './phone'
import { selectionTools, type ContextMenuEntry, type SelectionTools } from './context-menu'
import { t as shared } from '../locales/text'

/**
 * The text selection's toolbar (`web-application.md` § Context menus),
 * which the app mounts once at its root: while a selection holds text in
 * one object that offers tools for it, as a message does
 * (`selectionTools`), a row of its tools stands by it, Copy first, then the
 * object's, such as Send to Conversation. On the desktop it stands above
 * the selection, once the mouse that drags it is released; on a phone
 * under it, clear of the system's callout above. A tool with a submenu
 * opens it from its button, on a phone as a sheet; while it is open the
 * toolbar keeps the selection it opened for, though the focus moving into
 * its search field ends the selection on the page.
 */

const phone = usePhone()
const tools = shallowRef<SelectionTools | null>(null)
/** A mouse holds the selection it drags: the toolbar waits for its release. */
const dragging = ref(false)
/** The tool whose submenu is open, which holds the toolbar where it is. */
const opened = shallowRef<{ entry: ContextMenuEntry; button: HTMLElement } | null>(null)

let copiedText = ''
const { copy, copied } = useCopy(() => copiedText)

function read(): void {
  if (opened.value)
    return
  tools.value = selectionTools(window.getSelection(), (text) => {
    copiedText = text
    void copy()
  })
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

const shown = computed(() => tools.value !== null && !dragging.value)

const reference = computed(() => {
  const range = tools.value?.range
  if (!range)
    return null
  const start = range.startContainer
  return {
    contextElement: start instanceof Element ? start : start.parentElement ?? undefined,
    // Above the selection's first line on the desktop; under its last on a phone.
    getBoundingClientRect: () => {
      const rects = [...range.getClientRects()]
      return (phone.value ? rects.at(-1) : rects[0]) ?? range.getBoundingClientRect()
    },
  }
})

const floating = ref<HTMLElement>()
const { floatingStyles } = useFloating(reference, floating, {
  placement: computed(() => (phone.value ? 'bottom' : 'top')),
  strategy: 'fixed',
  // A phone's selection has its handles below its last line, which the toolbar clears.
  middleware: computed(() => [offset(phone.value ? 28 : 8), flip({ padding: 8 }), shift({ padding: 8 })]),
  whileElementsMounted: autoUpdate,
})
const z = stackAbove()

/** A tool chosen: one with a submenu opens it, any other acts. */
function choose(entry: ContextMenuEntry, event: Event): void {
  if (entry.submenu) {
    opened.value = { entry, button: event.currentTarget as HTMLElement }
    return
  }
  void entry.run?.()
}

function closeSubmenu(): void {
  opened.value = null
  read()
}
// A row chosen in the submenu ends it, as a menu's row does.
provide(menuRootKey, { dismiss: closeSubmenu })

const isCopy = (index: number) => index === 0
</script>

<template>
  <Teleport to="body">
    <!-- The toolbar takes no press from the selection: a press on it keeps the text selected. -->
    <div
      v-if="shown && tools"
      ref="floating"
      data-selection-toolbar
      class="overlay-shell flex items-center gap-0.5 rounded-lg p-1"
      :style="{ ...floatingStyles, zIndex: z() }"
      @mousedown.prevent
      @touchstart.prevent
    >
      <Button
        v-for="(entry, index) in tools.entries"
        :key="entry.label"
        size="sm"
        variant="ghost"
        :pressed="opened?.entry === entry"
        @pointerup="(event: PointerEvent) => event.pointerType === 'touch' && choose(entry, event)"
        @click="(event: MouseEvent) => choose(entry, event)"
      >
        <component :is="isCopy(index) && copied ? Check : entry.icon" v-if="entry.icon" :size="ICON_PX.in28" aria-hidden="true" />
        {{ isCopy(index) && copied ? shared('common.copied') : entry.label }}
      </Button>
    </div>
  </Teleport>
  <Popover
    v-if="opened?.entry.submenu"
    :overlay-store="appOverlayStore"
    is-open
    :anchor-el="opened.button"
    sheet
    @close="closeSubmenu"
  >
    <component :is="opened.entry.submenu" />
  </Popover>
</template>
