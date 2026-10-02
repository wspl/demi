<script setup lang="ts">
import { computed, onBeforeUnmount, shallowRef, watch } from 'vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import type { PanelTabKind } from '@demicodes/web-ui/agent/panel-kinds/kind'
import { bindPages, usePageHost, type BoundKinds } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { useProduct } from '../state/product'
import { useWorkPanel } from './work'

/**
 * The work panel beside one conversation: its saved tabs and pinned tabs from
 * the work store, and the kinds of the plugin pages the user has on, bound
 * to the conversation with each page's panel session.
 */
const props = defineProps<{ conversationId: string }>()
const emit = defineEmits<{ close: [] }>()

const product = useProduct()
const host = usePageHost()
const work = useWorkPanel()

const state = computed(() => work.stateFor(props.conversationId))

const recorded = computed(() => work.recorded(props.conversationId))

/**
 * The pages' kinds for this conversation (`plugin-pages.md` § Panel
 * sessions), for as long as the panel is open beside it with each plugin on.
 * A closed panel, or one beside a conversation its first send has not
 * created yet, holds no session and no view.
 */
const bound = shallowRef<BoundKinds | null>(null)
watch(
  () =>
    [
      props.conversationId,
      state.value.open,
      recorded.value,
      PLUGIN_PAGES.map((page) => pluginEnabled(product.snapshot, page.plugin)).join(),
    ] as const,
  ([conversationId, open, isRecorded]) => {
    bound.value?.dispose()
    bound.value = null
    if (!open || !isRecorded) {
      return
    }
    void work.load(conversationId)
    const pages = PLUGIN_PAGES.filter((page) => pluginEnabled(product.snapshot, page.plugin))
    bound.value = bindPages(pages, host, conversationId)
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  bound.value?.dispose()
})

const kinds = computed<PanelTabKind[]>(() => bound.value?.kinds ?? [])

/** Closing a tab removes it at once; its kind then does what a closed tab of it needs. */
function closeTabs(ids: string[]): void {
  const closing = state.value.panel.tabs.filter((tab) => ids.includes(tab.id))
  work.closeTabs(props.conversationId, ids)
  for (const tab of closing) {
    const kind = kinds.value.find((candidate) => candidate.kind === tab.kind)
    const parsed = kind?.schema.safeParse(tab.data)
    if (kind?.removed && parsed?.success) {
      kind.removed(parsed.data)
    }
  }
}
</script>

<template>
  <WorkPanel
    :panel="state.panel"
    :pinned="state.pinned"
    :kinds="kinds"
    :before-first-message="!recorded"
    @select="work.select(conversationId, $event)"
    @add-tab="(kind, data) => work.add(conversationId, kind, data)"
    @update-tab="(id, data) => work.update(conversationId, id, data)"
    @update-pinned="(kind, data) => work.updatePinned(conversationId, kind, data)"
    @close-tabs="closeTabs"
    @close="emit('close')"
  />
</template>
