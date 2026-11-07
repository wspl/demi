import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { Activity, CircleStop, SquareTerminal, X } from '@lucide/vue'
import type { ContextMenuGroup } from '../ui/context-menu'
import type { OwnWindow } from '../windows/SessionWindows.vue'
import { t as windowsText } from '../windows/locales/text'
import { commandKindOf, runningOfKind, terminalStatus, type CommandKind, type TerminalRecord } from './terminals'
import { t } from './locales/text'

/**
 * A conversation's Running and Services windows: each a tab for every running
 * command of its kind, oldest first, and the ended one it shows (`active`).
 * An ended command's tab closes, a running one's stays until the command is
 * ended. Each counts its running commands.
 *
 * With any tab, one is selected, as in a web browser: the command picked
 * while its tab stays, else the oldest, so a window never stands on no tab.
 */
export function useCommandWindows(
  terminals: () => readonly TerminalRecord[],
  end: (id: string) => void,
): {
  active: Ref<Record<CommandKind, string | null>>
  /** The tabs of the window of `kind`. */
  tabs: (kind: CommandKind) => TerminalRecord[]
  /** The tab the window of `kind` shows. */
  shown: (kind: CommandKind) => string | null
  windows: ComputedRef<OwnWindow[]>
} {
  const active = ref<Record<CommandKind, string | null>>({ command: null, service: null })

  function tabs(kind: CommandKind): TerminalRecord[] {
    const running = runningOfKind(terminals(), kind)
    const shown = terminals().find((terminal) => terminal.id === active.value[kind])
    return shown && !running.includes(shown) && commandKindOf(shown) === kind ? [...running, shown] : running
  }

  function shown(kind: CommandKind): string | null {
    const all = tabs(kind)
    return all.some((tab) => tab.id === active.value[kind]) ? active.value[kind] : all[0]?.id ?? null
  }

  function close(kind: CommandKind, id: string): void {
    active.value = { ...active.value, [kind]: tabs(kind).filter((tab) => tab.id !== id).at(-1)?.id ?? null }
  }

  function menu(kind: CommandKind, terminal: TerminalRecord): () => ContextMenuGroup[] {
    return () => [
      [{ icon: CircleStop, label: t('terminals.endCommand'), disabled: terminal.phase !== 'running' || terminal.stopping === true, run: () => end(terminal.id) }],
      [{ icon: X, label: t('terminals.closeTab'), disabled: terminal.phase === 'running', run: () => close(kind, terminal.id) }],
    ]
  }

  function window(kind: CommandKind, id: string, icon: typeof Activity, name: string): OwnWindow {
    return {
      id,
      icon,
      name,
      count: runningOfKind(terminals(), kind).length,
      anchors: true,
      tabs: tabs(kind).map((terminal) => ({
        id: terminal.id,
        title: terminal.title,
        busy: terminal.stopping === true,
        status: terminalStatus(terminal.phase),
        closable: terminal.phase !== 'running',
        menu: menu(kind, terminal),
      })),
      current: shown(kind),
      select: (tab) => {
        active.value = { ...active.value, [kind]: tab }
      },
      closeTab: (tab) => close(kind, tab),
    }
  }

  const windows = computed(() => [
    window('command', 'running', SquareTerminal, windowsText('names.running')),
    window('service', 'services', Activity, windowsText('names.services')),
  ])
  return { active, tabs, shown, windows }
}
