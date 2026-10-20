<script setup lang="ts">
import { computed } from 'vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import { usePageHost, useBoundKinds } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { useProduct } from '../state/product'
import { useWorkPanel } from './work'

/**
 * The work panel beside one conversation: its saved tabs and pinned tabs from
 * the work store, and the kinds of the plugin pages the user has on, bound
 * to the conversation.
 */
const props = defineProps<{ conversationId: string }>()
const emit = defineEmits<{ close: [] }>()

const product = useProduct()
const host = usePageHost()
const work = useWorkPanel()

const state = computed(() => work.stateFor(props.conversationId))

const recorded = computed(() => work.recorded(props.conversationId))

/**
 * The pages' kinds for this conversation (`packages/web-ui/src/plugins/page.ts`), while the panel
 * is open beside it, each page's while its plugin is on: turning one plugin on or off binds or lets
 * go of that page's alone. A closed panel, or one beside a conversation its first send has not
 * created yet, binds none; what a page keeps live is in its stores, which outlive the panel.
 */
const kinds = useBoundKinds(
  () => PLUGIN_PAGES.filter((page) => pluginEnabled(product.snapshot, page.plugin)),
  host,
  () => (state.value.open && recorded.value ? props.conversationId : null),
)

</script>

<template>
  <WorkPanel
    :panel="state.panel"
    :tabs-read="state.tabsRead"
    :pinned="state.pinned"
    :kinds="kinds"
    :before-first-message="!recorded"
    @select="work.select(conversationId, $event)"
    @move-tab="(id, index) => work.move(conversationId, id, index)"
    @update-tab="(id, data) => work.update(conversationId, id, data)"
    @update-pinned="(kind, data) => work.updatePinned(conversationId, kind, data)"
    @close-tabs="work.closeTabs(conversationId, $event)"
    @close="emit('close')"
  />
</template>
