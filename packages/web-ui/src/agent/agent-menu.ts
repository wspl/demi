import { CircleStop, Copy, SquareArrowOutUpRight } from '@lucide/vue'
import type { ContextMenuGroup } from '../ui/context-menu'
import { copyId } from '../infra/copy-id'
import { t as ui } from '../ui/locales/text'
import { t } from './locales/text'

/** An agent as its menu reads it: its number, and whether it runs now. */
export interface MenuAgent {
  number: number
  running: boolean
}

/**
 * An agent's menu, declared once: a
 * link to the agent offers Open, which acts as clicking the link does, then
 * Stop Agent, which only an agent that runs takes, and Copy Agent ID, the
 * number the `demi agent` commands take; its tab in the Agents window
 * offers the same without Open, and adds Close Tab.
 */
export function agentMenu(agent: MenuAgent, actions: { open?: () => void; stop: () => void }): ContextMenuGroup[] {
  return [
    ...(actions.open ? [[{ icon: SquareArrowOutUpRight, label: ui('contextMenu.open'), run: actions.open }]] : []),
    [
      { icon: CircleStop, label: t('subagents.stopAgent'), disabled: !agent.running, run: actions.stop },
      { icon: Copy, label: t('subagents.copyId'), run: () => copyId(String(agent.number)) },
    ],
  ]
}
