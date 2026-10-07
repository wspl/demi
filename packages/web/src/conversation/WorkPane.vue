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
/** Which pages the user has on, as one value that changes only when one is turned on or off. */
const enabledPages = computed(() => PLUGIN_PAGES.map((page) => pluginEnabled(product.snapshot, page.plugin)).join())
// Each source is compared on its own: any change of the product state, such as a summary the panel's
// revision changed, must not end the pages' sessions, which would remount every tab's content.
watch(
  [() => props.conversationId, () => state.value.open, recorded, enabledPages],
  ([conversationId, open, isRecorded]) => {
    bound.value?.dispose()
    bound.value = null
    if (!open || !isRecorded) {
      return
    }
    const pages = PLUGIN_PAGES.filter((page) => pluginEnabled(product.snapshot, page.plugin))
    bound.value = bindPages(pages, host, conversationId)
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  bound.value?.dispose()
})

const kinds = computed<PanelTabKind[]>(() => bound.value?.kinds ?? [])

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
    @close-tabs="work.closeTabs(conversationId, $event)"
    @close="emit('close')"
  />
</template>
