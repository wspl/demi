import type { Component } from 'vue'
import type { z } from 'zod'
import type { PanelTab } from '../panel-tabs'
import type { SentenceText, TitleText } from '../../ui/ui-text'

/** A command of a tab's menu that its kind offers, such as Reload. */
export interface TabCommand {
  label: TitleText
  icon?: Component
  /** It cannot run now, and shows disabled. */
  disabled?: boolean
  /** Why it cannot run, which its row says while it is disabled. */
  disabledReason?: SentenceText
  run(): void
}

/**
 * What the work panel needs to show the tabs of one kind
 * (`web-application.md` § Work panel). The panel knows nothing else about a
 * kind: the protocol, stream or routes its tabs use stay behind `content`.
 */
export interface PanelTabKind<Data = unknown> {
  kind: string
  /** A tab's `data`, checked where the saved state enters the page. */
  schema: z.ZodType<Data>
  // A method, so kinds of different data fit one list of kinds.
  title(data: Data): string
  /** The strip's mark. Props: `data`. */
  mark: Component
  /** A picture in place of the mark, such as a web page's icon, as an image URL; null while there is none. */
  icon?(data: Data, id: string): string | null
  /** Whether the tab `id` is at work, read reactively: its mark is a spinner meanwhile. */
  busy?(data: Data, id: string): boolean
  /**
   * The tab's content. Props: `tabId`, `data`, and `shown`, whether the tab is
   * the panel's selection. Emits `update` with the tab's next `data`, and
   * `close` to have the panel close the tab as its user would.
   */
  content: Component
  /**
   * The kind has one tab in every conversation's panel, ahead of the user's
   * tabs, which is never created, closed or saved; its data starts here and
   * lives in the page's memory. Its id is the kind's id.
   */
  pinned?: { data(): Data }
  /** What the tab's menu offers before its Close commands, read when the menu opens. */
  commands?(data: Data, id: string): readonly TabCommand[]
  /** The data of a copy of a tab, which its menu's Duplicate opens right after it; without it there is no Duplicate. */
  duplicate?(data: Data): Data
  /** What the strip shows after a pinned tab's title, such as its counts. Props: `data`. */
  badge?: Component
  /**
   * The kind is offered on the strip's new-tab control; `data` is a new
   * tab's. While `unavailable` gives a reason, read reactively, the control
   * is disabled and says it.
   */
  create?: {
    label: string
    icon: Component
    data(): Data
    unavailable?(): string | null
  }
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
