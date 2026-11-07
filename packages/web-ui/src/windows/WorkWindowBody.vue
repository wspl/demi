<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import DeviceOfflinePage from '../agent/DeviceOfflinePage.vue'
import PaneScope from '../agent/PaneScope.vue'
import { keptContents } from '../agent/panel-tabs'
import type { TabUpdate } from '../plugins/page'
import type { WorkWindow } from './work-windows'
import { provideWindowDismiss } from './window-dismiss'

/**
 * What a window of the conversation's work shows: its selected tab's content.
 *
 * A content once shown stays on the page, hidden and told it is not shown,
 * until its tab closes or the kinds are bound anew, as for another
 * conversation, so switching back to a tab reads nothing again. Only the
 * selected tab of a window on screen is told it is shown.
 *
 * While the conversation's device is offline, the window says so over its
 * contents, which stay mounted behind it.
 */
const props = defineProps<{
  window: WorkWindow
  /** Whether the window is on screen, not minimized. */
  onScreen: boolean
  deviceOffline: string | null
  /** What puts the window away, which a view's own gesture does on a phone; null elsewhere. */
  dismiss: (() => void) | null
}>()
provideWindowDismiss(() => props.dismiss)

const emit = defineEmits<{
  /** A tab's next data; `how` says when it is no step of the history (`TabUpdate`). */
  updateTab: [id: string, data: unknown, how?: TabUpdate]
  updatePinned: [kind: string, data: unknown, how?: TabUpdate]
  closeTab: [id: string]
}>()

const present = computed(() => props.window.tabs.map((tab) => tab.id))
const kept = shallowRef<readonly string[]>(keptContents([], props.window.selected, present.value))
/**
 * The contents shown first, as the window opened or its kinds were bound
 * anew: each says what it reads from its start (`PaneScope`'s `first`).
 */
const firstShown = shallowRef<ReadonlySet<string>>(new Set(kept.value))
watch([() => props.window.selected, present], ([selected, ids]) => {
  kept.value = keptContents(kept.value, selected, ids)
})

/**
 * A number for each kind as bound, which a pane's key holds: a kind bound anew
 * is a pane of its own, with its own first view.
 */
const bindings = new WeakMap<object, number>()
let bound = 0
function bindingOf(kind: object): number {
  let number = bindings.get(kind)
  if (number === undefined) {
    bound += 1
    number = bound
    bindings.set(kind, number)
  }
  return number
}

const contents = computed(() => kept.value.flatMap((id) => props.window.tabs.filter((tab) => tab.id === id)))
</script>

<template>
  <div class="flex h-full min-h-0 flex-col overflow-hidden">
    <DeviceOfflinePage v-if="deviceOffline !== null" :name="deviceOffline" />
    <div v-show="deviceOffline === null" class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <PaneScope
        v-for="tab in contents"
        v-show="tab.id === window.selected"
        :key="`${bindingOf(tab.kind)}:${tab.id}`"
        :name="tab.subject"
        :shown="onScreen && tab.id === window.selected"
        :first="firstShown.has(tab.id)"
        class="flex min-h-0 flex-1 flex-col"
      >
        <component
          :is="tab.kind.content"
          :tab-id="tab.id"
          :data="tab.data"
          :shown="onScreen && tab.id === window.selected"
          @update="(data: unknown, how?: TabUpdate) => tab.pinned ? emit('updatePinned', tab.id, data, how) : emit('updateTab', tab.id, data, how)"
          @close="tab.pinned || emit('closeTab', tab.id)"
        />
      </PaneScope>
    </div>
  </div>
</template>
