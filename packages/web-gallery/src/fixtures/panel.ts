/**
 * A conversation's work panel as the backend keeps it, and the browser
 * plugin's part in its tabs, in memory (`web-api.md` § Work panel state,
 * `live-view.md` § A browser tab in the panel): the gallery's panels change
 * the way the product's do, a beat late, over the gallery's own browser.
 */
import { BrowserTabsError, addedPanelTab, browserTabDataSchema, type BrowserTabData } from '@demicodes/plugin-browser/live/tabs'
import { applyPanelChange, type PanelAnswer, type PanelBackend, type PanelChange, type PanelRead } from '@demicodes/web-ui/agent/panel-changes'
import { openedTabIndex, type PanelTab } from '@demicodes/web-ui/agent/panel-tabs'
import type { AnyPluginPage } from '@demicodes/web-ui/plugins/page'
import type { BrowserTab } from '@demicodes/plugin-browser/generated/plugin'
import type { GalleryBrowser } from './live-browser'

/** How long the backend takes over a change of the panel. */
const CHANGE_MS = 120

function beat(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, CHANGE_MS))
}

/** What a change did to a tab, for the kind's plugin. */
type Told = { change: 'created' | 'removed'; tab: PanelTab }

export class GalleryPanel implements PanelBackend {
  private revision = 0
  private tabs: PanelTab[] = []
  /** Every id the panel had, which is never used again. */
  private readonly retired = new Set<string>()
  /** Told of a tab its user created or removed, once the change is answered, as a kind's plugin is. */
  told: ((told: Told) => void) | null = null
  /** Each change, the plugin's included, with the panel's revision, as the page's summary carries it. */
  changed: ((revision: number) => void) | null = null

  async read(): Promise<PanelRead> {
    await beat()
    return { revision: this.revision, tabs: this.tabs.map((tab) => ({ ...tab })) }
  }

  /**
   * A page's changes, as one change of the panel: the kind's plugin hears
   * of each tab its user created or removed.
   */
  async send(changes: readonly PanelChange[]): Promise<PanelAnswer> {
    await beat()
    const before = this.revision
    const told = changes.flatMap((change) => this.change(change) ?? [])
    if (this.revision > before) {
      // The request counts as one change, however many it made.
      this.revision = before + 1
      const revision = this.revision
      queueMicrotask(() => this.changed?.(revision))
    }
    for (const each of told) {
      queueMicrotask(() => this.told?.(each))
    }
    return { revision: this.revision, changed: this.revision > before }
  }

  /** Applies the plugin's `change`; answers what it did to a tab. */
  apply(change: PanelChange): Told | null {
    const before = this.revision
    const told = this.change(change)
    if (this.revision > before) {
      const revision = this.revision
      queueMicrotask(() => this.changed?.(revision))
    }
    return told
  }

  /** Applies `change`, counting it in the revision when it changes the tabs; answers what it did to a tab. */
  private change(change: PanelChange): Told | null {
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
    if (change.type === 'create') {
      return { change: 'created', tab: change.tab }
    }
    if (removed) {
      this.retired.add(removed.id)
      return { change: 'removed', tab: removed }
    }
    return null
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

/** The browser plugin's part in the panel's `browser` tabs, as the backend's plugin does it. */
export interface GalleryBrowserPlugin {
  /** Opens a browser tab for the panel tab, unless it shows one; answers the tab it shows then, as the plugin does. */
  bind(panelTab: string): Promise<string | null>
  /** Closes the browser tab of a panel tab its user removed. */
  removed(tab: PanelTab): Promise<void>
  /** Adds the agent's tabs, removes the ones closed on purpose and marks the ones the browser lost. */
  sync(): Promise<void>
}

/** `pages` are the panel's, whose kinds say which tab opened which. */
export function galleryBrowserPlugin(
  browser: GalleryBrowser,
  panel: GalleryPanel,
  pages: readonly AnyPluginPage[],
): GalleryBrowserPlugin {
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
        // The browser's or the Host's own code, as the backend's plugin keeps it.
        const code = error instanceof BrowserTabsError ? (error.code ?? 'failed') : 'failed'
        const message = error instanceof Error ? error.message : String(error)
        panel.apply({ type: 'update', id: panelTab, data: { failure: { code, message } } })
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
    sync: () => inTurn(async () => {
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
          // Beside its opener, as the backend's plugin places it, by the rule the page places Open Link in New Tab.
          const index = openedTabIndex(panel.all, pages, 'browser', data)
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
        } else if (present.shows > (data?.shows ?? 0)) {
          // The agent showed it: each page selects it once (`live-view.md` § Showing a tab).
          panel.apply({ type: 'update', id, data: { shows: present.shows } })
        }
      }
    }),
  }
}
