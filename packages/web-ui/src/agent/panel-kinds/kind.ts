import type { Component } from 'vue'
import type { z } from 'zod'
import type { PanelTab } from '../panel-tabs'

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
  /**
   * The tab's content. Props: `tabId`, `data`, and `shown`, whether the tab is
   * the panel's selection. Emits `update` with the tab's next `data`.
   */
  content: Component
  /** The user closed a tab of this kind; the panel has already removed it. */
  removed?(data: Data): void
  /** The kind is offered on the strip's new-tab control; `data` is a new tab's. */
  create?: {
    label: string
    icon: Component
    data(): Data
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
