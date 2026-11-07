<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import SubagentHistoryMenu from '@demicodes/web-ui/agent/SubagentHistoryMenu.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import TerminalPanel from '@demicodes/web-ui/agent/TerminalPanel.vue'
import { useAgentsWindow } from '@demicodes/web-ui/agent/agents-window'
import { useCommandWindows } from '@demicodes/web-ui/agent/command-windows'
import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { CostShown } from '@demicodes/web-ui/agent/context-cost'
import SessionWindows from '@demicodes/web-ui/windows/SessionWindows.vue'
import { useWindowStage } from '@demicodes/web-ui/windows/stage'
import { useAgentContext } from '../agent-context'

/**
 * The product's Agents, Running and Services windows over a specimen's
 * children and commands, with the taskbar and its system buttons, over what
 * the default slot holds, as the product's stand over the conversation.
 *
 * `open` and `activeId` are the Agents window's: a receipt or a link sets
 * them as the product's session does, and the window's own Close and tabs set
 * them back. `shows` opens Running or Services as the specimen comes. With
 * `fill`, a window that opens fills the area, for a specimen of the window
 * alone.
 */
const props = withDefaults(defineProps<{
  agents?: readonly SubagentRecord[]
  terminals?: readonly TerminalRecord[]
  reveal?: TranscriptReveal | null
  streamed?: boolean
  readLatest?: (id: string) => Promise<void>
  agentCost?: (id: string) => CostShown | null
  output?: (id: string) => RequestOutputReading | null
  shows?: 'running' | 'services'
  fill?: boolean
}>(), { agents: () => [], terminals: () => [] })
const emit = defineEmits<{
  abort: []
  abortAgent: [id: string]
  abortTerminal: [id: string]
  revealed: []
}>()
const open = defineModel<boolean>('open', { default: false })
const activeId = defineModel<string | null>('activeId', { default: null })

defineSlots<{
  default?(): unknown
  /** The system area's menus after its buttons, such as Schedules. */
  system?(): unknown
}>()

const agentContext = useAgentContext()
const agentsWindow = useAgentsWindow(() => props.agents, {
  stop: (id) => emit('abortAgent', id),
  stopAll: () => emit('abort'),
})
const commandWindows = useCommandWindows(() => props.terminals, (id) => emit('abortTerminal', id))
watch(activeId, (id) => {
  agentsWindow.activeId.value = id
}, { immediate: true })
watch(agentsWindow.activeId, (id) => {
  activeId.value = id
})

const stage = useWindowStage()
const area = ref<InstanceType<typeof SessionWindows> | null>(null)
function showing(id: string): boolean {
  const placed = stage.find(id)
  return placed !== undefined && placed.placement !== 'minimized'
}
function show(id: string): void {
  area.value?.show(id)
}
// With `fill`, a window fills the area the first time it opens.
const seen = new Set<string>()
watch(() => stage.state.windows.map((window) => window.id), (ids) => {
  for (const id of ids.filter((each) => !seen.has(each))) {
    seen.add(id)
    if (props.fill && stage.find(id)?.placement !== 'minimized') {
      stage.maximize(id)
    }
  }
})
function follow(shown: boolean): void {
  if (shown && !showing('agents')) {
    show('agents')
  } else if (!shown && stage.find('agents')) {
    stage.remove('agents')
  }
}
onMounted(() => {
  if (props.shows) {
    show(props.shows)
  }
  follow(open.value)
})
watch(open, follow)
watch(() => showing('agents'), (shown) => {
  open.value = shown
})
</script>

<template>
  <SessionWindows ref="area" :stage="stage" :own="[agentsWindow.window.value, ...commandWindows.windows.value]" :bottom="0">
    <slot />
    <template #own="{ id }">
      <SubagentPanel
        v-if="id === 'agents'"
        v-model:active-id="agentsWindow.activeId.value"
        :agents="agents"
        :terminals="terminals"
        :reveal="reveal"
        :streamed="streamed"
        :read-latest="readLatest"
        :agent-cost="agentCost"
        :output="output"
        @revealed="emit('revealed')"
        v-bind="agentContext.panel"
      />
      <TerminalPanel
        v-else
        :terminals="commandWindows.tabs(id === 'running' ? 'command' : 'service')"
        :active-id="commandWindows.shown(id === 'running' ? 'command' : 'service')"
      />
    </template>
    <template #trailing="{ id }">
      <SubagentHistoryMenu
        v-if="id === 'agents'"
        :agents="agents"
        :active-id="agentsWindow.activeId.value"
        @select="(agent) => (agentsWindow.activeId.value = agent)"
      />
    </template>
    <template #system>
      <slot name="system" />
    </template>
  </SessionWindows>
</template>
