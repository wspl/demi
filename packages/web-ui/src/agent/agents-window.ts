import { computed, h, ref, type ComputedRef, type WritableComputedRef } from 'vue'
import { Bot, CircleStop, X } from '@lucide/vue'
import type { ContextMenuGroup } from '../ui/context-menu'
import { ICON_PX } from '../ui/icon-metrics'
import type { OwnWindow } from '../windows/SessionWindows.vue'
import { t as windowsText } from '../windows/locales/text'
import AgentMark from './AgentMark.vue'
import { agentMenu } from './agent-menu'
import { agentName, subagentPanelTabs, type SubagentRecord } from './subagents'
import { t } from './locales/text'

/**
 * A conversation's Agents window: a tab for each running child, and the
 * finished one it shows (`activeId`). A finished child's tab closes, a running
 * one's stays until the child is stopped. Its count is the running children's.
 *
 * With any tab, one is selected, as in a web browser: the child picked while
 * its tab stays, else the first tab, so the window never stands on no tab.
 */
export function useAgentsWindow(
  agents: () => readonly SubagentRecord[],
  actions: { stop: (id: string) => void; stopAll: () => void },
): { activeId: WritableComputedRef<string | null>; window: ComputedRef<OwnWindow> } {
  const picked = ref<string | null>(null)
  const tabs = computed(() => subagentPanelTabs(agents(), picked.value))
  const activeId = computed<string | null>({
    get: () => (tabs.value.some((tab) => tab.id === picked.value) ? picked.value : tabs.value[0]?.id ?? null),
    set: (id) => {
      picked.value = id
    },
  })

  function close(id: string): void {
    activeId.value = tabs.value.filter((tab) => tab.id !== id).at(-1)?.id ?? null
  }

  /** An agent's tab: its agent's menu, a link's less Open, then Close Tab once the agent is done. */
  function tabMenu(id: string): () => ContextMenuGroup[] {
    return () => {
      const agent = tabs.value.find((each) => each.id === id)
      if (!agent) {
        return []
      }
      return [
        ...agentMenu({ number: agent.number, running: agent.phase === 'running' }, { stop: () => actions.stop(id) }),
        [{ icon: X, label: t('subagents.closeTab'), disabled: agent.phase === 'running', run: () => close(id) }],
      ]
    }
  }

  const window = computed<OwnWindow>(() => {
    const running = agents().filter((agent) => agent.phase === 'running')
    return {
      id: 'agents',
      icon: Bot,
      name: windowsText('names.agents'),
      count: running.length,
      anchors: true,
      tabs: tabs.value.map((agent) => ({
        id: agent.id,
        title: agentName(agent.name, agent.number),
        mark: { render: () => h(AgentMark, { phase: agent.phase, size: ICON_PX.markIn28 }) },
        closable: agent.phase !== 'running',
        menu: tabMenu(agent.id),
      })),
      current: activeId.value,
      // What acts on every agent.
      menu: () => [[{ icon: CircleStop, label: t('subagents.stopAll'), disabled: running.length === 0, run: actions.stopAll }]],
      select: (tab) => {
        activeId.value = tab
      },
      closeTab: close,
    }
  })
  return { activeId, window }
}
