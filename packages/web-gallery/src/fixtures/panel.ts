/**
 * A conversation's work panel as the backend keeps it, and the browser
 * plugin's part in its tabs, in memory (`web-api-conversations.md` § Work panel state): the gallery's panels change
 * the way the product's do, a beat late, over the gallery's own browser.
 */
import { BrowserTabsError, addedPanelTab, browserTabDataSchema, type BrowserTabData, type TabFailure } from '@demicodes/plugin-browser/live/tabs'
import { SERVER_FAILED } from './plugins'
import { applyPanelChange, type PanelAnswer, type PanelBackend, type PanelChange, type PanelRead } from '@demicodes/web-ui/agent/panel-changes'
import type { PanelTab } from '@demicodes/web-ui/agent/panel-tabs'
import type { BrowserTab } from '@demicodes/plugin-browser/generated/plugin'
import type { GalleryBrowser } from './live-browser'

/** How long the backend takes over a change of the panel. */
const CHANGE_MS = 120

function beat(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, CHANGE_MS))
}


export class GalleryPanel implements PanelBackend {
  private revision = 0
  private tabs: PanelTab[] = []
  /** Every id the panel had, which is never used again. */
  private readonly retired = new Set<string>()
  /**
   * Told of a tab its user removed, once the change is answered, as a kind's
   * plugin is; a tab the user created is none of the plugin's business.
   */
  removed: ((tab: PanelTab) => void) | null = null
  /** Each change, the plugin's included, with the panel's revision, as the page's summary carries it. */
  changed: ((revision: number) => void) | null = null

  async read(): Promise<PanelRead> {
    await beat()
    return { revision: this.revision, tabs: this.tabs.map((tab) => ({ ...tab })) }
  }

  /**
   * A page's changes, as one change of the panel: the kind's plugin hears
   * of each tab its user removed.
   */
  async send(changes: readonly PanelChange[]): Promise<PanelAnswer> {
    await beat()
    const before = this.revision
    const removed = changes.flatMap((change) => this.change(change) ?? [])
    if (this.revision > before) {
      // The request counts as one change, however many it made.
      this.revision = before + 1
      const revision = this.revision
      queueMicrotask(() => this.changed?.(revision))
    }
    for (const tab of removed) {
      queueMicrotask(() => this.removed?.(tab))
    }
    return { revision: this.revision, changed: this.revision > before }
  }

  /** Applies the plugin's `change`, which no plugin is told of. */
  apply(change: PanelChange): void {
    const before = this.revision
    this.change(change)
    if (this.revision > before) {
      const revision = this.revision
      queueMicrotask(() => this.changed?.(revision))
    }
  }

  /** Applies `change`, counting it in the revision when it changes the tabs; answers the tab it removed, if any. */
  private change(change: PanelChange): PanelTab | null {
    if (change.type === 'create' && (this.retired.has(change.tab.id) || this.tab(change.tab.id))) {
      return null
    }
    const removed = change.type === 'remove' ? this.tab(change.id) : null
    const next = applyPanelChange(this.tabs, change)
    if (JSON.stringify(next) === JSON.stringify(this.tabs)) {
      return null
    }
    this.tabs = next
    this.revision += 1
    if (removed) {
      this.retired.add(removed.id)
    }
    return removed
  }

  tab(id: string): PanelTab | null {
    return this.tabs.find((tab) => tab.id === id) ?? null
  }

  get all(): readonly PanelTab[] {
    return this.tabs
  }
}

/** The data of a `browser` tab; none for one that does not fit, which the plugin leaves as it is. */
function browserData(tab: PanelTab | null): BrowserTabData | null {
  const parsed = browserTabDataSchema.safeParse(tab?.data)
  return parsed.success ? parsed.data : null
}

/** The browser tab a panel tab shows, while the browser has it. */
function live(data: BrowserTabData): string | undefined {
  return data.closed ? undefined : data.tab
}

/**
 * The panel tab that shows the browser tab whose page opened `tab`; none for
 * a tab no page opened, or whose opener the panel does not show.
 */
function openerFor(panel: GalleryPanel, tab: BrowserTab): string | undefined {
  if (tab.createdBy.kind !== 'page') {
    return undefined
  }
  const opener = tab.createdBy.opener
  return panel.all.find((each) => {
    const data = each.kind === 'browser' ? browserData(each) : null
    return data !== null && live(data) === opener
  })?.id
}

/**
 * Where a tab `opener` opened goes: right after it, behind the tabs it opened
 * before that still stand right after it, as Chrome places the tabs a link
 * opens and the backend's plugin places them; after the others (undefined)
 * when the panel no longer has the opener.
 */
function openedIndex(panel: GalleryPanel, opener: string): number | undefined {
  const tabs = panel.all
  const at = tabs.findIndex((tab) => tab.id === opener)
  if (at < 0) {
    return undefined
  }
  let end = at + 1
  while (end < tabs.length && browserData(tabs[end]!)?.openedBy === opener) {
    end += 1
  }
  return end
}

/** The browser plugin's part in the panel's `browser` tabs, as the backend's plugin does it. */
export interface GalleryBrowserPlugin {
  /** Opens a browser tab for the panel tab, unless it shows one; answers the tab it shows then, as the plugin does. */
  bind(panelTab: string): Promise<string | null>
  /** Closes the browser tab of a panel tab its user removed. */
  removed(tab: PanelTab): Promise<void>
  /**
   * The browser told the plugin its tabs changed: adds the agent's and the
   * pages' tabs, removes the ones closed on purpose and marks the ones the browser lost.
   */
  tabsChanged(): Promise<void>
  /** Resolves once the plugin did the work it was given so far. */
  settled(): Promise<void>
}

export function galleryBrowserPlugin(browser: GalleryBrowser, panel: GalleryPanel): GalleryBrowserPlugin {
  // One piece of the conversation's work at a time, as the plugin does it.
  let turn: Promise<void> = Promise.resolve()
  function inTurn<T>(work: () => Promise<T>): Promise<T> {
    const run = turn.then(work)
    // The next piece waits for this one however it ends; its caller hears how.
    turn = run.then(() => {}, () => {})
    return run
  }

  return {
    bind: (panelTab) => inTurn(async () => {
      const data = browserData(panel.tab(panelTab))
      if (!data) {
        return null
      }
      const shown = live(data)
      if (shown !== undefined && !data.failure) {
        return shown
      }
      const asked = data.url
      let opened
      try {
        opened = await browser.open(asked)
      } catch (error) {
        // The browser's or the Host's own reason, as the backend's plugin keeps
        // it, or the status a call would answer a failure without one with.
        const message = error instanceof Error ? error.message : String(error)
        const reason = error instanceof BrowserTabsError ? error.reason : null
        const failure: TabFailure = reason ? { code: reason, message } : { status: SERVER_FAILED, message }
        panel.apply({ type: 'update', id: panelTab, data: { failure } })
        return null
      }
      panel.apply({ type: 'update', id: panelTab, data: { tab: opened.id, closed: null, failure: null } })
      const now = browserData(panel.tab(panelTab))
      if (!now) {
        await browser.close(opened.id)
        return null
      }
      if (now.url !== asked) {
        await browser.navigate(opened.id, now.url)
      }
      return opened.id
    }),
    removed: (tab) => inTurn(async () => {
      const data = browserData(tab)
      const shown = data ? live(data) : undefined
      if (shown !== undefined) {
        await browser.close(shown)
      }
    }),
    settled: () => turn,
    tabsChanged: () => inTurn(async () => {
      const listed = browser.listed.value.tabs
      const bound = panel.all
        .filter((tab) => tab.kind === 'browser')
        .map((tab) => ({ id: tab.id, data: browserData(tab) }))
      const shown = new Set(bound.map((tab) => tab.data?.tab))
      for (const tab of listed) {
        if (tab.createdBy.kind !== 'user' && !shown.has(tab.id)) {
          const data: BrowserTabData = { url: tab.url, tab: tab.id, title: tab.title }
          if (tab.shows > 0) {
            data.shows = tab.shows
          }
          const opener = openerFor(panel, tab)
          if (opener !== undefined) {
            data.openedBy = opener
          }
          // Beside its opener, as the backend's plugin places it.
          const index = opener === undefined ? undefined : openedIndex(panel, opener)
          panel.apply({ type: 'create', tab: { id: addedPanelTab(tab.id), kind: 'browser', data }, index })
        }
      }
      for (const { id, data } of bound) {
        const tab = data ? live(data) : undefined
        if (tab === undefined) {
          continue
        }
        const present = listed.find((candidate) => candidate.id === tab)
        if (!present && browser.closedOnPurpose.has(tab)) {
          panel.apply({ type: 'remove', id })
        } else if (!present) {
          panel.apply({ type: 'update', id, data: { closed: true } })
        } else {
          // The tab follows its page's address and title, and the agent's
          // showings, which each page selects once.
          const follows: Partial<BrowserTabData> = {}
          if (present.url !== data?.url) {
            follows.url = present.url
          }
          if (present.title !== data?.title) {
            follows.title = present.title
          }
          if (present.shows > (data?.shows ?? 0)) {
            follows.shows = present.shows
          }
          if (Object.keys(follows).length > 0) {
            panel.apply({ type: 'update', id, data: follows })
          }
        }
      }
    }),
  }
}
