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

const emit = defineEmits<{
  /** The close on a running tab: stop that command. */
  abort: [id: string]
}>()
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

// Closing a running tab stops the command; closing an exited one puts it away.
function closeTab(terminal: TerminalRecord): void {
  if (terminal.phase === 'running') {
    emit('abort', terminal.id)
    return
  }
  activeId.value = tabs.value.filter((tab) => tab.id !== terminal.id).at(-1)?.id ?? null
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
        @pointerdown="activate(terminal.id)"
        @close="closeTab(terminal)"
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
