<script setup lang="ts">
import { computed } from 'vue'
import { CircleStop, X } from '@lucide/vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
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
  /** End Command in a running tab's menu: stop that command. */
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

function terminalOf(id: string): TerminalRecord | undefined {
  return tabs.value.find((terminal) => terminal.id === id)
}

// An ended command's tab puts it away; a running one's stays until the command is ended.
function closeTab(id: string): void {
  activeId.value = tabs.value.filter((tab) => tab.id !== id).at(-1)?.id ?? null
}
</script>

<template>
  <SessionOverlay
    :open="tabs.length > 0"
    :dismiss-outside="dismissOutside"
    @close="closePanel"
  >
    <template #tabs="{ openTabMenu }">
      <TabItem
        v-for="terminal in tabs"
        :key="terminal.id"
        :title="terminal.title"
        :is-active="terminal.id === active?.id"
        :status="terminalStatus(terminal.phase)"
        mark="terminal"
        :closable="false"
        @select="activate(terminal.id)"
        @contextmenu="openTabMenu($event, terminal.id)"
      />
    </template>
    <template #tabMenu="{ id }">
      <MenuItem
        :icon="CircleStop"
        label="End Command"
        :disabled="terminalOf(id)?.phase !== 'running'"
        @select="emit('abort', id)"
      />
      <MenuDivider />
      <MenuItem
        :icon="X"
        label="Close Tab"
        :disabled="terminalOf(id)?.phase === 'running'"
        @select="closeTab(id)"
      />
    </template>
    <XtermView
      v-if="active"
      :key="active.id"
      :output="active.output"
      :chars="active.chars"
      :running="active.phase === 'running'"
      :script="active.script"
    />
  </SessionOverlay>
</template>
