import { computed, shallowRef, type ComputedRef } from 'vue'
import type { PanelTab } from './panel-tabs'

/**
 * One change of the work panel's tabs (`web-api.md` § Work panel state). An
 * update sets the fields of `data` it names and removes the null ones.
 */
export type PanelChange =
  | { type: 'create'; tab: PanelTab; index?: number }
  | { type: 'update'; id: string; data: Record<string, unknown> }
  | { type: 'remove'; id: string }
  | { type: 'move'; id: string; index: number }

/** The backend's panel: its tabs, and how many changes made them. */
export interface PanelRead {
  revision: number
  tabs: PanelTab[]
}

/** Where the panel's tabs are kept: the backend's routes, or a fixture's. */
export interface PanelBackend {
  read(): Promise<PanelRead>
  /** Applies the change and answers the panel's revision once it is in it. */
  send(change: PanelChange): Promise<{ revision: number }>
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

/**
 * The work panel's tabs as a page shows them (`web-application.md` § Work
 * panel): the panel it last read, with its own changes that are not in it yet
 * applied on top. A change shows at once and is sent in order, one at a time;
 * it leaves once a panel at least as new as its answer was read, so a panel
 * read after a removal never brings the tab back. A change the backend refuses
 * leaves at once, and the panel shows as the backend has it.
 */
export class PanelTabs {
  private readonly confirmed = shallowRef<PanelRead | null>(null)
  /** The page's changes not yet in the panel it read, oldest first, with the revision each answer named. */
  private readonly pending = shallowRef<{ change: PanelChange; answered: number | null }[]>([])
  private sending = false
  private started = false
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

  /** Shows `change` at once and sends it after the changes before it. */
  change(change: PanelChange): void {
    this.pending.value = [...this.pending.value, { change, answered: null }]
    void this.flush()
  }

  /** Reads the panel again, as when its revision rose. */
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
        const entry = this.pending.value.find((candidate) => candidate.answered === null)
        if (!entry) {
          return
        }
        let answer: { revision: number }
        try {
          answer = await this.backend.send(entry.change)
        } catch (error) {
          this.pending.value = this.pending.value.filter((candidate) => candidate !== entry)
          this.refused(error)
          continue
        }
        entry.answered = answer.revision
        if (answer.revision > this.revision) {
          await this.refresh()
        }
        this.prune()
      }
    } finally {
      this.sending = false
    }
  }
}
