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
 * A window the conversation's tabs open in, such as Files or the browser: its
 * id, unique across plugins, its name and its icon, which its title bar and
 * its taskbar button show.
 */
export interface PanelWindow {
  id: string
  name(): string
  icon: Component
  /**
   * Whether the taskbar's Open lists it; absent, it does. A window that only
   * shows what another place opens in it, such as Attachments, which an
   * attachment in the transcript opens, says false.
   */
  openable?: boolean
}

/**
 * What the conversation's windows need to show the tabs of one kind. They
 * know nothing else about a kind: the protocol, stream or routes its tabs use
 * stay behind `content`.
 */
export interface PanelTabKind<Data = unknown> {
  kind: string
  /** A tab's `data`, checked where the saved state enters the page. */
  schema: z.ZodType<Data>
  /** The window its tabs open in. */
  window: PanelWindow
  /** The data of a new tab the window's + opens; absent, it opens tabs only by intents. */
  start?(): Data
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
  /**
   * The rows of its window's entry in the taskbar's Open, as a submenu, such
   * as the simulators a device has. Emits `opened` once the user chose one,
   * which shows the window.
   */
  openMenu?: Component
  /**
   * What its window shows over its tabs while the kind opens one, such as a
   * simulator that boots; nothing while none opens. Emits `failed` when the
   * opening was refused, which closes the window if it holds no tab.
   */
  opening?: Component
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
