import { computed, shallowRef, type ComputedRef } from 'vue'
import { removeTabs, type PanelTab } from './panel-tabs'

/**
 * One change of the work panel's tabs (`web-api.md` § Work panel state). An
 * update sets the fields of `data` it names and removes the null ones.
 */
export type PanelChange =
  | { type: 'create'; tab: PanelTab; index?: number }
  | { type: 'update'; id: string; data: Record<string, unknown> }
  | { type: 'remove'; id: string }
  | { type: 'move'; id: string; index: number }

/** The place right after the tab `id` among `tabs`, where a tab opened from it goes; undefined without it. */
export function indexAfter(tabs: readonly PanelTab[], id: string): number | undefined {
  const at = tabs.findIndex((tab) => tab.id === id)
  return at < 0 ? undefined : at + 1
}

/** The backend's panel: its tabs, and how many changes made them. */
export interface PanelRead {
  revision: number
  tabs: PanelTab[]
}

/** What a request of changes answers: the panel's revision once they are in it, and whether they changed it. */
export interface PanelAnswer {
  revision: number
  changed: boolean
}

/** Where the panel's tabs are kept: the backend's routes, or a fixture's. */
export interface PanelBackend {
  read(): Promise<PanelRead>
  /**
   * Applies the changes in order, all or none, as one change of the panel,
   * and answers the panel's revision once they are in it and whether they
   * changed it.
   */
  send(changes: readonly PanelChange[]): Promise<PanelAnswer>
}

/** `tabs` after `change`, as the backend applies it; a change with nothing to do leaves them. */
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

/** The update that turns `before` into `after`: each field that changed, and each removed one as null. */
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

/** A change of the page's that is not in the panel it read yet: waiting to go, sent, or answered with the revision it named. */
interface Pending {
  change: PanelChange
  answered: number | null
  sent: boolean
}

/**
 * The work panel's tabs as a page shows them (`web-application.md` § Work
 * panel): the panel it last read, with its own changes that are not in it yet
 * applied on top. A change shows at once; the changes made while none is on
 * its way go together, in order, as one request, so Close Others is one
 * request (`web-application.md` § Requests for one action). An answer that
 * changed the panel one revision past the one the page holds is the panel
 * with the request in it; the panel is read only when an answer, or the
 * summary, names a revision past the one the page then holds, which says
 * another change came in between (`web-api.md` § Work panel state). A change leaves once a panel at least as new as its
 * answer is held, so a panel read after a removal never brings the tab back.
 * A request the backend refuses leaves at once, and the panel shows as the
 * backend has it.
 */
export class PanelTabs {
  private readonly confirmed = shallowRef<PanelRead | null>(null)
  /** The page's changes not yet in the panel it holds, oldest first. */
  private readonly pending = shallowRef<Pending[]>([])
  private sending = false
  private started = false
  /** The newest revision the page heard of: an answer's, or the summary's. */
  private known = -1
  /** The tabs the page shows. */
  readonly tabs: ComputedRef<PanelTab[]>

  constructor(
    private readonly backend: PanelBackend,
    /** Reports a change the backend refused, or a read that failed. */
    private readonly refused: (error: unknown) => void,
  ) {
    this.tabs = computed(() =>
      this.pending.value.reduce((tabs, entry) => applyPanelChange(tabs, entry.change), this.confirmed.value?.tabs ?? []),
    )
  }

  /** The revision of the panel last read; -1 before the first read. */
  get revision(): number {
    return this.confirmed.value?.revision ?? -1
  }

  /** Whether the panel was read once. */
  get read(): boolean {
    return this.confirmed.value !== null
  }

  /**
   * Starts reading and sending: before it, as for a conversation that has no
   * backend record yet, changes only show.
   */
  start(): void {
    if (this.started) {
      return
    }
    this.started = true
    void this.refresh()
    void this.flush()
  }

  /** Shows `changes` at once and sends them, together, after the changes before them. */
  change(...changes: PanelChange[]): void {
    this.pending.value = [...this.pending.value, ...changes.map((change) => ({ change, answered: null, sent: false }))]
    void this.flush()
  }

  /**
   * Hears of the panel's revision, as the conversation's summary carries
   * it: a panel read once is read again when the revision is past the one
   * it holds, unless a request of the page's is on its way, whose answer
   * may be that revision.
   */
  noticed(revision: number): void {
    this.known = Math.max(this.known, revision)
    if (!this.sending && this.read && this.known > this.revision) {
      void this.refresh()
    }
  }

  /** Reads the panel again. */
  async refresh(): Promise<void> {
    try {
      this.receive(await this.backend.read())
    } catch (error) {
      this.refused(error)
    }
  }

  /** Takes a panel read anywhere: an older one than the panel held changes nothing. */
  receive(panel: PanelRead): void {
    if (this.confirmed.value !== null && panel.revision <= this.confirmed.value.revision) {
      return
    }
    this.confirmed.value = panel
    this.prune()
  }

  private prune(): void {
    const revision = this.revision
    this.pending.value = this.pending.value.filter((entry) => entry.answered === null || entry.answered > revision)
  }

  private async flush(): Promise<void> {
    if (this.sending || !this.started) {
      return
    }
    this.sending = true
    try {
      for (;;) {
        const batch = this.pending.value.filter((entry) => !entry.sent)
        if (batch.length === 0) {
          return
        }
        for (const entry of batch) {
          entry.sent = true
        }
        let answer: PanelAnswer
        try {
          answer = await this.backend.send(batch.map((entry) => entry.change))
        } catch (error) {
          this.pending.value = this.pending.value.filter((entry) => !batch.includes(entry))
          this.refused(error)
          continue
        }
        for (const entry of batch) {
          entry.answered = answer.revision
        }
        this.known = Math.max(this.known, answer.revision)
        const held = this.confirmed.value
        if (held !== null && answer.changed && answer.revision === held.revision + 1) {
          // The request alone made this revision: the panel is the one held with it.
          this.confirmed.value = {
            revision: answer.revision,
            tabs: batch.reduce((tabs, entry) => applyPanelChange(tabs, entry.change), held.tabs),
          }
        }
        // Another change came in between, or after: only then is the panel read.
        if (held !== null && this.known > this.revision) {
          await this.refresh()
        }
        this.prune()
      }
    } finally {
      this.sending = false
    }
  }
}

/** A panel's selection history, the tab or kind it selected last at its end, which closing tabs changes. */
export interface PanelHistory {
  history: readonly string[]
}

/**
 * Closes the tabs `ids` as one change of the panel, one request, as Close
 * Others is (`web-application.md` § Requests for one action); the panel
 * shows what was selected before a closed one.
 */
export function closePanelTabs(selection: PanelHistory, tabs: PanelTabs, ids: readonly string[]): void {
  selection.history = removeTabs({ history: selection.history, tabs: tabs.tabs.value }, ids).history
  tabs.change(...ids.map((id) => ({ type: 'remove' as const, id })))
}

/** Whether `data` is an object a kind's tab keeps as its `data`. */
function isData(data: unknown): data is Record<string, unknown> {
  return data !== null && typeof data === 'object' && !Array.isArray(data)
}

/** The tab's next `data`, from its kind: only what changed is sent; a tab gone, or data that is no object, sends nothing. */
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
