import type { CallEditSelection, ChangeFile, ChangeMode } from '../files/changes'
import { baseName } from '../files/paths'

/**
 * One tab in the work panel: a file to read, or the changes (diffs), of
 * which there is at most one. The panel shows tabs for the conversation on
 * screen; the host keeps them per conversation and swaps the set when the
 * conversation changes. Each tab walks what it has shown with Back and
 * Forward: a file tab its files, the change tab its steps, each a mode and
 * the file shown in it, so Back can cross modes.
 */
export type WorkTab = FileWorkTab | ChangeWorkTab

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
export function fileWorkTab(id: string, path: string): WorkTab {
  return { id, kind: 'file', path, back: [], forward: [] }
}

/** The change tab opened in `mode`, with nothing to go back or forward to. */
export function changeWorkTab(id: string, mode: ChangeMode): WorkTab {
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
  newId: () => string,
): { tabs: WorkTab[]; activeId: string } {
  const existing = findChangeWorkTab(tabs)
  const tab = existing ?? changeWorkTab(newId(), 'conversation')
  const opened = existing ? tabs : [...tabs, tab]
  return {
    tabs: showChangeInTab(opened, tab.id, 'conversation', null, { call: selection, edit: 0 }),
    activeId: tab.id,
  }
}

/**
 * Shows `path` in the active file tab, in place: the tab turns into that
 * file and remembers the one it showed. A tab already showing the file is
 * activated instead, so a file has one tab. Without an active file tab
 * (a change tab, or none) a new file tab opens.
 */
export function showFileInTab(
  tabs: readonly WorkTab[],
  activeId: string | null,
  path: string,
  newId: () => string,
): { tabs: WorkTab[]; activeId: string | null } {
  const shown = tabs.find((tab) => tab.kind === 'file' && tab.path === path)
  if (shown) {
    return { tabs: [...tabs], activeId: shown.id }
  }
  const active = tabs.find((tab) => tab.id === activeId)
  if (active?.kind !== 'file') {
    const tab = fileWorkTab(newId(), path)
    return { tabs: [...tabs, tab], activeId: tab.id }
  }
  const replaced: WorkTab = { ...active, path, back: [...active.back, active.path], forward: [] }
  return { tabs: tabs.map((tab) => (tab.id === active.id ? replaced : tab)), activeId: active.id }
}

/** The tab showing what it showed before, the current step kept for Forward. */
export function goBackInTab(tabs: readonly WorkTab[], id: string): WorkTab[] {
  return tabs.map((tab) => {
    if (tab.id !== id || tab.back.length === 0) {
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
    if (tab.id !== id || tab.forward.length === 0) {
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

/** What the tab is labelled with: the file's name, or Change. */
export function workTabTitle(tab: WorkTab): string {
  return tab.kind === 'file' ? baseName(tab.path) : 'Change'
}

/** The one change tab, when it is open. */
export function findChangeWorkTab(tabs: readonly WorkTab[]): ChangeWorkTab | null {
  return tabs.find((tab): tab is ChangeWorkTab => tab.kind === 'change') ?? null
}

/**
 * The tabs and active tab after `ids` close. When the active tab goes, the
 * nearest remaining tab before it takes over, else the first that remains.
 */
export function closeWorkTabs(
  tabs: readonly WorkTab[],
  activeId: string | null,
  ids: readonly string[],
): { tabs: WorkTab[]; activeId: string | null } {
  const closing = new Set(ids)
  const remaining = tabs.filter((tab) => !closing.has(tab.id))
  if (activeId !== null && !closing.has(activeId)) {
    return { tabs: remaining, activeId }
  }
  const activeIndex = tabs.findIndex((tab) => tab.id === activeId)
  const before = tabs
    .slice(0, Math.max(0, activeIndex))
    .reverse()
    .find((tab) => !closing.has(tab.id))
  const next = before ?? remaining[0] ?? null
  return { tabs: remaining, activeId: next?.id ?? null }
}
