import { computed, ref } from 'vue'
import { firstInspectSubagentId, type SubagentRecord } from './subagents'
import { firstRunningTerminalId, type TerminalRecord } from './terminals'

/** The dock can inspect one child agent or one terminal at a time. */
export function useSessionPanels(
  agents: () => readonly SubagentRecord[],
  terminals: () => readonly TerminalRecord[],
) {
  const panel = ref<{
    kind: 'agent' | 'terminal'
    id: string
  } | null>(null)
  const activeSubagentId = computed({
    get: () => (panel.value?.kind === 'agent' ? panel.value.id : null),
    set: (id: string | null) => {
      panel.value = id
        ? {
            kind: 'agent',
            id,
          }
        : null
    },
  })
  const activeTerminalId = computed({
    get: () => (panel.value?.kind === 'terminal' ? panel.value.id : null),
    set: (id: string | null) => {
      panel.value = id
        ? {
            kind: 'terminal',
            id,
          }
        : null
    },
  })
  function toggleAgents(): void {
    activeSubagentId.value = activeSubagentId.value
      ? null
      : firstInspectSubagentId(agents())
  }
  function toggleTerminals(): void {
    activeTerminalId.value = activeTerminalId.value
      ? null
      : firstRunningTerminalId(terminals())
  }
  function close(): void {
    panel.value = null
  }
  return {
    activeSubagentId,
    activeTerminalId,
    toggleAgents,
    toggleTerminals,
    close,
  }
}
