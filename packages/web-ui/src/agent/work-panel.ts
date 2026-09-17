import type { CallEditSelection, ChangeFile, ChangeMode } from '../files/changes'
import { baseName } from '../files/paths'
import { closeTabs } from './tab-close'

/** Fixed Change and File tabs and removable browser tabs for one conversation. */
export type WorkTab = FileWorkTab | ChangeWorkTab | BrowserWorkTab

export interface BrowserWorkTab {
  id: string
  kind: 'browser'
  title: string
  address: string
}

/** Initial fixed tabs for a conversation's work panel. */
export function workPanelTabs(path = ''): WorkTab[] {
  return [changeWorkTab('change', 'uncommitted'), fileWorkTab('file', path)]
}

/** Add an independent browser address draft and select its tab. */
export function addBrowserTab(tabs: readonly WorkTab[]): { tabs: WorkTab[]; activeId: string } {
  const tab: BrowserWorkTab = { id: crypto.randomUUID(), kind: 'browser', title: 'New tab', address: '' }
  return { tabs: [...tabs, tab], activeId: tab.id }
}

/** Close browser tabs while preserving the fixed Change and File tabs. */
export function closeBrowserTabs(tabs: readonly WorkTab[], activeId: string | null, ids: readonly string[]) {
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
    if (tab.id !== id || tab.kind === 'browser' || tab.back.length === 0) {
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
    if (tab.id !== id || tab.kind === 'browser' || tab.forward.length === 0) {
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

/** The section label, or the selected file's basename. */
export function workTabTitle(tab: WorkTab): string {
  return tab.kind === 'file' ? baseName(tab.path) : tab.kind === 'change' ? 'Change' : tab.title
}

/** The one change tab, when it is open. */
export function findChangeWorkTab(tabs: readonly WorkTab[]): ChangeWorkTab | null {
  return tabs.find((tab): tab is ChangeWorkTab => tab.kind === 'change') ?? null
}
