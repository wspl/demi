import type { LiveTab } from '@demicodes/browser-protocol/live'
import type { CallEditSelection, ChangeFile, ChangeMode } from '../files/changes'
import { baseName } from '../files/paths'
import { closeTabs } from './tab-close'

/**
 * Fixed Change and File tabs, the conversation browser's own tabs, and the
 * local browser tabs of one conversation
 * (`web-application.md` § Package responsibilities).
 */
export type WorkTab = FileWorkTab | ChangeWorkTab | BrowserWorkTab | HostWorkTab

/** A tab of the conversation's browser, shown live. */
export interface HostWorkTab {
  id: string
  kind: 'host'
  tab: LiveTab
}

/** Work-panel IDs of Host tabs are the browser's own, prefixed. */
export function hostWorkTabId(tab: string): string {
  return `host:${tab}`
}

/**
 * The work tabs with the browser's own tabs first, as the live view reports
 * them; tabs that closed on the Host go with them.
 */
export function withHostTabs(tabs: readonly WorkTab[], live: readonly LiveTab[]): WorkTab[] {
  const host: WorkTab[] = live.map((tab) => ({ id: hostWorkTabId(tab.id), kind: 'host', tab }))
  return [...host, ...tabs.filter((tab) => tab.kind !== 'host')]
}

export interface BrowserWorkTab {
  id: string
  kind: 'browser'
  title: string
  /** The address bar's draft; it becomes `url` when the user submits it. */
  address: string
  /** The page the tab shows in its frame; null while the tab is empty. */
  url: string | null
  /** The page is an expose opened from the session tools; a submitted address ends that. */
  expose: boolean
}

/** A page a browser tab opens on: an expose's URL under its address as the title. */
export interface BrowserPage {
  url: string
  title: string
  expose: boolean
}

/** Initial fixed tabs for a conversation's work panel. */
export function workPanelTabs(path = ''): WorkTab[] {
  return [changeWorkTab('change', 'uncommitted'), fileWorkTab('file', path)]
}

/** Add a browser tab and select it: empty with its own address draft, or showing `page`. */
export function addBrowserTab(tabs: readonly WorkTab[], page?: BrowserPage): { tabs: WorkTab[]; activeId: string } {
  const tab: BrowserWorkTab = {
    id: crypto.randomUUID(),
    kind: 'browser',
    title: page?.title ?? 'New tab',
    address: page?.url ?? '',
    url: page?.url ?? null,
    expose: page?.expose ?? false,
  }
  return { tabs: [...tabs, tab], activeId: tab.id }
}

/**
 * The tab after the user submits its address draft: an `http` or `https` URL
 * loads, a draft without a scheme is tried as `https`, and anything else
 * leaves the tab as it is.
 */
export function loadBrowserAddress(tab: BrowserWorkTab): BrowserWorkTab {
  const draft = tab.address.trim()
  const candidate = draft.includes('://') ? draft : `https://${draft}`
  if (!draft || !URL.canParse(candidate)) {
    return tab
  }
  const url = new URL(candidate)
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return tab
  }
  return { ...tab, title: url.host, address: url.href, url: url.href, expose: false }
}

/** Close browser tabs while preserving the fixed Change and File tabs. */
export function closeBrowserTabs(tabs: readonly WorkTab[], activeId: string | null, ids: readonly string[]) {
  // Host tabs close on the Host itself, not here.
  const browserIds = tabs.filter((tab) => tab.kind === 'browser' && ids.includes(tab.id)).map((tab) => tab.id)
  return closeTabs(tabs, activeId, browserIds)
}

export interface FileWorkTab {
  id: string
  kind: 'file'
  /** The working-tree path the tab shows. */
  path: string
  /** The paths shown before, latest last; Back returns to them. */
  back: string[]
  /** The paths left by Back, latest last; Forward returns to them. */
  forward: string[]
}

export interface ChangeWorkTab {
  id: string
  kind: 'change'
  mode: ChangeMode
  /** The working-tree path to show; null selects its first listed file. */
  uncommitted: string | null
  call: CallEditSelection | null
  edit: number
  back: ChangeStep[]
  forward: ChangeStep[]
}

/** A history step identifies the mode, call, file and edit segment. */
export interface ChangeStep {
  mode: ChangeMode
  uncommitted: string | null
  call: CallEditSelection | null
  edit: number
}

/** A file tab showing `path`, with nothing to go back or forward to. */
export function fileWorkTab(id: string, path: string): FileWorkTab {
  return { id, kind: 'file', path, back: [], forward: [] }
}

/** The change tab opened in `mode`, with nothing to go back or forward to. */
export function changeWorkTab(id: string, mode: ChangeMode): ChangeWorkTab {
  return { id, kind: 'change', mode, uncommitted: null, call: null, edit: 0, back: [], forward: [] }
}

/** Conversation paths come from the picked file; only the working tree holds a list selection. */
export function changeTabPath(tab: ChangeWorkTab, mode = tab.mode, files?: readonly ChangeFile[]): string | null {
  if (mode === 'conversation') {
    return tab.call?.file.path ?? null
  }
  if (!files || files.some((file) => file.path === tab.uncommitted)) {
    return tab.uncommitted
  }
  return files[0]?.path ?? null
}

function currentChangeStep(tab: ChangeWorkTab): ChangeStep {
  return { mode: tab.mode, uncommitted: tab.uncommitted, call: tab.call, edit: tab.edit }
}

function atChangeStep(tab: ChangeWorkTab, step: ChangeStep): ChangeWorkTab {
  return { ...tab, ...step }
}

/**
 * The change tab showing `path` in `mode`, remembering what it showed: a
 * mode switch and a pick in the tree are both steps Back returns to.
 */
export function showChangeInTab(tabs: readonly WorkTab[], id: string, mode: ChangeMode, path: string | null, selection?: { call: CallEditSelection | null; edit: number }): WorkTab[] {
  return tabs.map((tab) => {
    if (tab.id !== id || tab.kind !== 'change') {
      return tab
    }
    const call = selection ? selection.call : tab.call
    const nextPath = mode === 'conversation' ? call?.file.path ?? null : path
    const sameFile = tab.mode === mode && changeTabPath(tab, mode) === nextPath
      && tab.call?.commandId === call?.commandId
    const edit = selection?.edit ?? (sameFile ? tab.edit : 0)
    if (sameFile && tab.edit === edit) {
      return tab
    }
    const step: ChangeStep = {
      mode,
      uncommitted: mode === 'uncommitted' ? path : tab.uncommitted,
      call,
      edit,
    }
    return {
      ...atChangeStep(tab, step),
      back: [...tab.back, currentChangeStep(tab)],
      forward: [],
    }
  })
}

/** A file pill selects its call in the single change tab, creating that tab if needed. */
export function showCallEdit(
  tabs: readonly WorkTab[],
  selection: CallEditSelection,
): { tabs: WorkTab[]; activeId: string } {
  const existing = findChangeWorkTab(tabs)
  const tab = existing ?? changeWorkTab('change', 'conversation')
  const opened = existing ? tabs : [...tabs, tab]
  return {
    tabs: showChangeInTab(opened, tab.id, 'conversation', null, { call: selection, edit: 0 }),
    activeId: tab.id,
  }
}

/** Shows a workspace file in the single File section, retaining its navigation history. */
export function showFileInTab(
  tabs: readonly WorkTab[],
  path: string,
): { tabs: WorkTab[]; activeId: string | null } {
  const active = tabs.find((tab) => tab.kind === 'file')
  if (!active) {
    const tab = fileWorkTab('file', path)
    return { tabs: [...tabs, tab], activeId: tab.id }
  }
  if (active.path === path) {
    return { tabs: [...tabs], activeId: active.id }
  }
  const replaced: WorkTab = {
    ...active,
    path,
    back: active.path ? [...active.back, active.path] : active.back,
    forward: [],
  }
  return { tabs: tabs.map((tab) => tab.id === active.id ? replaced : tab), activeId: active.id }
}

/** The tab showing what it showed before, the current step kept for Forward. */
export function goBackInTab(tabs: readonly WorkTab[], id: string): WorkTab[] {
  return tabs.map((tab) => {
    if (tab.id !== id || tab.kind === 'browser' || tab.kind === 'host' || tab.back.length === 0) {
      return tab
    }
    if (tab.kind === 'file') {
      return { ...tab, path: tab.back.at(-1)!, back: tab.back.slice(0, -1), forward: [...tab.forward, tab.path] }
    }
    return {
      ...atChangeStep(tab, tab.back.at(-1)!),
      back: tab.back.slice(0, -1),
      forward: [...tab.forward, currentChangeStep(tab)],
    }
  })
}

/** The tab showing what Back left, the current step kept for Back. */
export function goForwardInTab(tabs: readonly WorkTab[], id: string): WorkTab[] {
  return tabs.map((tab) => {
    if (tab.id !== id || tab.kind === 'browser' || tab.kind === 'host' || tab.forward.length === 0) {
      return tab
    }
    if (tab.kind === 'file') {
      return { ...tab, path: tab.forward.at(-1)!, forward: tab.forward.slice(0, -1), back: [...tab.back, tab.path] }
    }
    return {
      ...atChangeStep(tab, tab.forward.at(-1)!),
      forward: tab.forward.slice(0, -1),
      back: [...tab.back, currentChangeStep(tab)],
    }
  })
}

/** The section label, the tab's title, or the selected file's basename. */
export function workTabTitle(tab: WorkTab): string {
  if (tab.kind === 'file') {
    return baseName(tab.path)
  }
  if (tab.kind === 'change') {
    return 'Change'
  }
  if (tab.kind === 'browser') {
    return tab.title
  }
  return tab.tab.title || hostTabAddress(tab.tab) || 'New tab'
}

/** A Host tab's address as the strip and the bar show it. */
export function hostTabAddress(tab: LiveTab): string {
  const url = URL.parse(tab.url)
  return url ? url.host || url.href : tab.url
}

/** The one change tab, when it is open. */
export function findChangeWorkTab(tabs: readonly WorkTab[]): ChangeWorkTab | null {
  return tabs.find((tab): tab is ChangeWorkTab => tab.kind === 'change') ?? null
}
