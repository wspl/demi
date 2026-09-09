<script setup lang="ts">
import { computed } from 'vue'
import SessionOverlay from './SessionOverlay.vue'
import TabItem from './TabItem.vue'
import XtermView from './XtermView.vue'
import { terminalPanelTabs, terminalStatus, type TerminalRecord } from './terminals'

const props = withDefaults(
  defineProps<{
    terminals: readonly TerminalRecord[]
    dismissOutside?: boolean
  }>(),
  {
    dismissOutside: true,
  },
)

const activeId = defineModel<string | null>('activeId', { required: true })
const tabs = computed(() => terminalPanelTabs(props.terminals, activeId.value))
const active = computed(
  () =>
    tabs.value.find((terminal) => terminal.id === activeId.value) ??
    tabs.value[0] ??
    null,
)
function activate(id: string): void {
  activeId.value = id
}

function closePanel(): void {
  activeId.value = null
}
</script>

<template>
  <SessionOverlay
    :open="tabs.length > 0"
    :dismiss-outside="dismissOutside"
    @close="closePanel"
  >
    <template #tabs>
      <TabItem
        v-for="terminal in tabs"
        :key="terminal.id"
        :tab="{ id: terminal.id, title: terminal.name }"
        :is-active="terminal.id === active?.id"
        :status="terminalStatus(terminal.phase)"
        mark="terminal"
        :closable="false"
        @pointerdown="activate(terminal.id)"
      />
    </template>
    <XtermView
      v-if="active"
      :key="active.id"
      :output="active.output"
      :running="active.phase === 'running'"
    />
  </SessionOverlay>
</template>
