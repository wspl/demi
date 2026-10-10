import { computed, ref } from 'vue'
import { firstRunningSubagentId, type SubagentRecord } from './subagents'
import { firstRunningTerminalId, type TerminalRecord } from './terminals'

/**
 * The dock opens one window at a time: the Agents panel, on one child or on
 * none, or the Running panel on one terminal (`subagents.md` § Product
 * rendering).
 */
export function useSessionPanels(
  agents: () => readonly SubagentRecord[],
  terminals: () => readonly TerminalRecord[],
) {
  const panel = ref<
    | { kind: 'agent'; id: string | null }
    | { kind: 'terminal'; id: string }
    | null
  >(null)
  /** The Agents panel is open; it opens on the first running child, or empty when none runs. */
  const agentsOpen = computed({
    get: () => panel.value?.kind === 'agent',
    set: (open: boolean) => {
      panel.value = open ? { kind: 'agent', id: firstRunningSubagentId(agents()) } : null
    },
  })
  /** The child the Agents panel shows; setting one opens the panel on it, and null leaves it open, empty. */
  const activeSubagentId = computed({
    get: () => (panel.value?.kind === 'agent' ? panel.value.id : null),
    set: (id: string | null) => {
      panel.value = { kind: 'agent', id }
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
    agentsOpen.value = !agentsOpen.value
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
    agentsOpen,
    activeSubagentId,
    activeTerminalId,
    toggleAgents,
    toggleTerminals,
    close,
  }
}
