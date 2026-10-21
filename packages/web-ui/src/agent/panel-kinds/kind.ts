import type { Component } from 'vue'
import type { z } from 'zod'
import type { PanelTab } from '../panel-tabs'
import type { TitleText } from '../../ui/ui-text'

/** A command of a tab's menu that its kind offers, such as Reload. */
export interface TabCommand {
  label: TitleText
  icon?: Component
  /** It cannot run now, and shows disabled. */
  disabled?: boolean
  run(): void
}

/**
 * What the work panel needs to show the tabs of one kind. The panel knows
 * nothing else about a kind: the protocol, stream or routes its tabs use stay
 * behind `content`.
 */
export interface PanelTabKind<Data = unknown> {
  kind: string
  /** A tab's `data`, checked where the saved state enters the page. */
  schema: z.ZodType<Data>
  // A method, so kinds of different data fit one list of kinds.
  title(data: Data): string
  /** What the pane says it reads while it waits, such as a file's name; absent, the title. */
  subject?(data: Data): string
  /** The strip's mark. Props: `data`. */
  mark: Component
  /**
   * A picture in place of the mark, such as a web page's icon, as an image URL;
   * null while there is none.
   */
  icon?(data: Data, id: string): string | null
  /**
   * Whether the tab `id` is at work, read reactively: its mark is a spinner
   * meanwhile.
   */
  busy?(data: Data, id: string): boolean
  /**
   * The tab's content. Props: `tabId`, `data`, and `shown`, whether the tab is
   * the panel's selection. Emits `update` with the tab's next `data` and,
   * for a change that is no step of the history, a `TabUpdate` saying so;
   * and `close` to have the panel close the tab as its user would.
   */
  content: Component
  /**
   * The kind has one tab in every conversation's panel, ahead of the user's
   * tabs, which is never created, closed or saved; its data starts here and
   * lives in the page's memory. Its id is the kind's id.
   */
  pinned?: { data(): Data }
  /**
   * What the tab's menu offers before its Close commands, read when the menu
   * opens.
   */
  commands?(data: Data, id: string): readonly TabCommand[]
  /**
   * What the strip shows after a pinned tab's title, such as its counts. Props:
   * `data`.
   */
  badge?: Component
}

/** A tab with its kind and checked data, or why the panel cannot show it. */
export type ResolvedPanelTab =
  | { tab: PanelTab; kind: PanelTabKind; data: unknown; title: string }
  | { tab: PanelTab; kind: null; data: null; title: string }

/**
 * The tab as the panel shows it. A tab of an unknown kind, or whose data does
 * not fit its kind, stays in the strip and says so: the page never repairs or
 * drops a saved tab.
 */
export function resolvePanelTab(tab: PanelTab, kinds: readonly PanelTabKind[]): ResolvedPanelTab {
  const kind = kinds.find((candidate) => candidate.kind === tab.kind)
  if (!kind) {
    return { tab, kind: null, data: null, title: tab.kind }
  }
  const parsed = kind.schema.safeParse(tab.data)
  if (!parsed.success) {
    return { tab, kind: null, data: null, title: tab.kind }
  }
  return { tab, kind, data: parsed.data, title: kind.title(parsed.data) }
}
