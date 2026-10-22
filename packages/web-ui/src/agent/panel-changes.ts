import { computed, type ComputedRef } from 'vue'
import { Slot, type SlotChange } from '../infra/slot'
import { removeTabs, type PanelTab } from './panel-tabs'

/**
 * One change of the work panel's tabs. An update sets the fields of `data` it
 * names and removes the null ones. /
 */
export type PanelChange =
  | { type: 'create'; tab: PanelTab; index?: number }
  | { type: 'update'; id: string; data: Record<string, unknown> }
  | { type: 'remove'; id: string }
  | { type: 'move'; id: string; index: number }

/**
 * The backend's panel: its tabs, and how many changes made them. It comes with
 * the conversation's entry in the list, so the panel draws as soon as the
 * conversation is listed
 * (`docs/decisions/0049-the-work-panel-shows-at-once.md`).
 */
export interface PanelRead {
  revision: number
  tabs: PanelTab[]
}

/**
 * What a request of changes answers: the panel's revision once they are in it,
 * and whether they changed it.
 */
export interface PanelAnswer {
  revision: number
  changed: boolean
}

/**
 * Where the panel's changes go: the backend's route, or a fixture's. The
 * panel itself comes to the page as its owner announces it
 * (`PanelTabs.receive`), never by a read.
 */
export interface PanelBackend {
  /**
   * Applies the changes in order, all or none, as one change of the panel,
   * and answers the panel's revision once they are in it and whether they
   * changed it.
   */
  send(changes: readonly PanelChange[]): Promise<PanelAnswer>
}

/**
 * `tabs` after `change`, as the backend applies it; a change with nothing to do
 * leaves them.
 */
export function applyPanelChange(tabs: readonly PanelTab[], change: PanelChange): PanelTab[] {
  const index = (id: string) => tabs.findIndex((tab) => tab.id === id)
  switch (change.type) {
    case 'create': {
      if (index(change.tab.id) >= 0) {
        return [...tabs]
      }
      const at = Math.min(change.index ?? tabs.length, tabs.length)
      return [...tabs.slice(0, at), change.tab, ...tabs.slice(at)]
    }
    case 'update':
      return tabs.map((tab) => (tab.id === change.id ? { ...tab, data: mergeData(tab.data, change.data) } : tab))
    case 'remove':
      return tabs.filter((tab) => tab.id !== change.id)
    case 'move': {
      const from = index(change.id)
      if (from < 0) {
        return [...tabs]
      }
      const moving = tabs[from]!
      const others = tabs.filter((tab) => tab.id !== change.id)
      const at = Math.min(change.index, others.length)
      return [...others.slice(0, at), moving, ...others.slice(at)]
    }
  }
}

/** `data` with each field of `fields` set, and each null one removed. */
export function mergeData(data: unknown, fields: Record<string, unknown>): Record<string, unknown> {
  const merged: Record<string, unknown> = data !== null && typeof data === 'object' ? { ...data } : {}
  for (const [field, value] of Object.entries(fields)) {
    if (value === null) {
      delete merged[field]
    } else {
      merged[field] = value
    }
  }
  return merged
}

/**
 * The update that turns `before` into `after`: each field that changed, and
 * each removed one as null.
 */
export function dataChanges(before: unknown, after: Record<string, unknown>): Record<string, unknown> {
  const previous: Record<string, unknown> = before !== null && typeof before === 'object' ? { ...before } : {}
  const fields: Record<string, unknown> = {}
  for (const [field, value] of Object.entries(after)) {
    if (JSON.stringify(previous[field]) !== JSON.stringify(value)) {
      fields[field] = value
    }
  }
  for (const field of Object.keys(previous)) {
    if (!Object.hasOwn(after, field)) {
      fields[field] = null
    }
  }
  return fields
}

/**
 * A change of the page's that is not in the panel it read yet, and the request
 * it will go in.
 */
interface Unsent {
  change: PanelChange
  shown: SlotChange<PanelTab[]>
}

/**
 * The work panel's tabs as a page shows them: a slot (`infra/slot`) of the
 * panel it last read, its revision the panel's, with the page's own changes
 * that are not in it yet on top.
 *
 * - A change shows at once. The changes made while none is on its way go
 *   together, in order, as one request, so Close Others is one request.
 * - An answer that changed the panel one revision past the one the page holds
 *   is the panel with the request in it. Any other answer waits for the panel
 *   it names, which the conversation's entry in the list brings.
 * - A change leaves once a panel at least as new as its answer is held, so a
 *   panel received after a removal never brings the tab back.
 * - A request the backend refuses leaves at once, and the panel shows as the
 *   backend has it.
 */
export class PanelTabs {
  /** Before the first read the panel is empty, so a change shows over it. */
  private readonly slot = new Slot<PanelTab[]>([])
  /** The changes made and not yet sent, oldest first. */
  private unsent: Unsent[] = []
  private sending = false
  private started = false
  /** The tabs the page shows. */
  readonly tabs: ComputedRef<PanelTab[]>

  constructor(
    private readonly backend: PanelBackend,
    /** Reports a change the backend refused. */
    private readonly refused: (error: unknown) => void,
  ) {
    this.tabs = computed(() => this.slot.shown.value ?? [])
  }

  /** The revision of the panel last received; -1 before the first. */
  get revision(): number {
    return this.slot.revision()
  }

  /** Whether the panel was received once. */
  get read(): boolean {
    return this.slot.read.value !== null
  }

  /**
   * Starts sending: before it, as for a conversation that has no backend
   * record yet, changes only show.
   */
  start(): void {
    if (this.started) {
      return
    }
    this.started = true
    void this.flush()
  }

  /**
   * Shows `changes` at once and sends them, together, after the changes before
   * them.
   */
  change(...changes: PanelChange[]): void {
    this.unsent.push(...changes.map((change) => ({
      change,
      shown: this.slot.change((tabs) => applyPanelChange(tabs, change)),
    })))
    void this.flush()
  }

  /**
   * Takes the panel as its owner announced it, such as in the
   * conversation's entry in the list: one no newer than the panel held
   * changes nothing.
   */
  receive(panel: PanelRead): void {
    this.slot.replace(panel.tabs, { revisions: { '': panel.revision } })
  }

  private async flush(): Promise<void> {
    if (this.sending || !this.started) {
      return
    }
    this.sending = true
    try {
      for (;;) {
        const batch = this.unsent.splice(0)
        if (batch.length === 0) {
          return
        }
        const shown = batch.map((entry) => entry.shown)
        this.slot.send(shown)
        let answer: PanelAnswer
        try {
          answer = await this.backend.send(batch.map((entry) => entry.change))
        } catch (error) {
          this.slot.drop(shown)
          this.refused(error)
          continue
        }
        // A request that alone made the next revision leaves the panel as the
        // tabs held with it; any other answer waits for the panel it names.
        const alone = this.read && answer.changed && answer.revision === this.revision + 1
        this.slot.land(shown, {
          revisions: { '': answer.revision },
          ...(alone ? { value: (tabs: PanelTab[]) => batch.reduce((next, entry) => applyPanelChange(next, entry.change), tabs) } : {}),
        })
      }
    } finally {
      this.sending = false
    }
  }
}

/**
 * A panel's selection history, the tab or kind it selected last at its end,
 * which closing tabs changes.
 */
export interface PanelHistory {
  history: readonly string[]
}

/**
 * Closes the tabs `ids` as one change of the panel, one request, like Close
 * Others; the panel shows what was selected before a closed one.
 */
export function closePanelTabs(selection: PanelHistory, tabs: PanelTabs, ids: readonly string[]): void {
  selection.history = removeTabs({ history: selection.history, tabs: tabs.tabs.value }, ids).history
  tabs.change(...ids.map((id) => ({ type: 'remove' as const, id })))
}

/** Whether `data` is an object a kind's tab keeps as its `data`. */
function isData(data: unknown): data is Record<string, unknown> {
  return data !== null && typeof data === 'object' && !Array.isArray(data)
}

/**
 * The tab's next `data`, from its kind: only what changed is sent; a tab gone,
 * or data that is no object, sends nothing.
 */
export function updatePanelTab(tabs: PanelTabs, id: string, data: unknown): void {
  const current = tabs.tabs.value.find((tab) => tab.id === id)
  if (!current || !isData(data)) {
    return
  }
  const fields = dataChanges(current.data, data)
  if (Object.keys(fields).length > 0) {
    tabs.change({ type: 'update', id, data: fields })
  }
}
