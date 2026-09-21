/**
 * The `browser` tab kind's own side of the work panel
 * (`browser-live-view.md` § A browser tab in the panel): what a tab saves, the
 * requests that list, open, close and navigate the conversation browser's
 * tabs, and the one view a page keeps while a `browser` tab is shown.
 */
import { nextTick, shallowRef, type ShallowRef } from 'vue'
import { z } from 'zod'
import { browserCreatedBySchema } from '@demicodes/browser-protocol'
import { viewerClipboard } from './clipboard'
import { viewerPlatform } from './input'
import { LiveSession, type OpenLiveStream } from './session'

/** What a new tab shows before the user goes anywhere. */
export const NEW_TAB_URL = 'about:blank'

export const browserTabDataSchema = z.object({
  /** The address the tab shows, saved as the page changes it. */
  url: z.string(),
  /** The conversation browser's tab this panel tab is bound to, once it has one. */
  tab: z.string().optional(),
})
export type BrowserTabData = z.infer<typeof browserTabDataSchema>

export const browserTabInfoSchema = z.object({
  id: z.string(),
  title: z.string(),
  url: z.string(),
  createdBy: browserCreatedBySchema,
})
export type BrowserTabInfo = z.infer<typeof browserTabInfoSchema>

export const browserTabListSchema = z.object({
  tabs: z.array(browserTabInfoSchema),
})
export type BrowserTabList = z.infer<typeof browserTabListSchema>

/** A request the backend or the browser refused, with the answer's own code and message. */
export class BrowserTabsError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message)
  }
}

/**
 * The conversation browser's tab routes (`web-api.md` § Conversation browser
 * tabs) and its user stream, as the product or the gallery supplies them.
 * Every request rejects with a `BrowserTabsError`.
 */
export interface BrowserTabsApi {
  list(): Promise<BrowserTabList>
  open(url: string): Promise<BrowserTabInfo>
  close(tab: string): Promise<void>
  navigate(tab: string, url: string): Promise<void>
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<void>
  stream: OpenLiveStream
  /** A user operation, which the product reports as conversation activity. */
  onOperation?: () => void
}

/** The panel's tab state, as far as a kind may touch it. */
export interface BrowserPanelTabs {
  /** The `data` of every `browser` tab the panel has. */
  bound(): BrowserTabData[]
  /** A tab after the others, without taking the selection. */
  add(data: BrowserTabData): void
}

/** Answers that will not change by asking again. */
const FINAL_CODES = new Set(['conversation_not_found', 'conversation_archived'])

/**
 * One conversation's browser, for one page: the last tab list it read, and
 * the view, open only while a `browser` tab's content is shown.
 */
export class BrowserTabsController {
  /** The last list read; null before the first answer. */
  readonly list: ShallowRef<BrowserTabList | null> = shallowRef(null)
  /** Why the last list could not be read, until one is. */
  readonly listError: ShallowRef<BrowserTabsError | null> = shallowRef(null)
  readonly session: ShallowRef<LiveSession | null> = shallowRef(null)
  /** Opens in flight by panel tab, so a content shown twice asks once. */
  private readonly opening = new Map<string, Promise<BrowserTabInfo>>()
  /** Browser tabs whose panel tab the user closed, until the browser has closed them too. */
  private readonly leaving = new Set<string>()
  private readonly showing = new Set<string>()
  private closing: ReturnType<typeof setTimeout> | null = null
  private disposed = false

  constructor(
    readonly api: BrowserTabsApi,
    private readonly panel: BrowserPanelTabs,
  ) {}

  /**
   * Reads the browser's tabs and adds those no panel tab is bound to. Nothing
   * is ever removed here: a tab the Host lost stays the user's to close.
   */
  async refresh(): Promise<void> {
    let list: BrowserTabList
    try {
      list = await this.api.list()
    } catch (error) {
      if (this.disposed) {
        return
      }
      this.listError.value = asTabsError(error)
      if (FINAL_CODES.has(this.listError.value.code)) {
        this.closeView()
      }
      return
    }
    if (this.disposed) {
      return
    }
    this.listError.value = null
    this.adopt(list)
  }

  /**
   * A list from a request or from the view's `state`. While a tab is being
   * opened the list may already name it, before its panel tab is bound: it
   * would be taken for the agent's. Adding waits for the open to settle.
   */
  adopt(list: BrowserTabList): void {
    this.list.value = list
    if (this.opening.size > 0) {
      return
    }
    const bound = new Set(this.panel.bound().map((data) => data.tab))
    for (const tab of list.tabs) {
      // A tab on its way out is still listed until the browser closes it; it is not the agent's.
      if (!bound.has(tab.id) && !this.leaving.has(tab.id)) {
        this.panel.add({ url: tab.url, tab: tab.id })
      }
    }
  }

  /**
   * Opens the browser tab a panel tab asked for and hands it to `bind`, which
   * writes it into the panel tab. Asking again joins the request in flight.
   */
  open(panelTabId: string, url: string, bind: (tab: BrowserTabInfo) => void): Promise<BrowserTabInfo> {
    const pending = this.opening.get(panelTabId)
    if (pending) {
      return pending
    }
    const opened = this.request(panelTabId, url, bind)
    this.opening.set(panelTabId, opened)
    return opened
  }

  private async request(panelTabId: string, url: string, bind: (tab: BrowserTabInfo) => void): Promise<BrowserTabInfo> {
    try {
      const tab = await this.api.open(url)
      if (!this.disposed) {
        // The browser has the tab now, whatever the last list said.
        const known = this.list.value?.tabs.filter((item) => item.id !== tab.id) ?? []
        this.list.value = { tabs: [...known, tab] }
        bind(tab)
      }
      return tab
    } finally {
      this.opening.delete(panelTabId)
      const list = this.list.value
      if (list && !this.disposed) {
        this.adopt(list)
      }
    }
  }

  /**
   * The panel tab is gone; its browser tab goes with it. The request waits
   * until the panel has shown its next selection: the view then watches that
   * tab already, so closing this one takes nothing from under it, and a close
   * moves the picture exactly as a click on the next tab does. A tab the
   * browser already lost is closed.
   */
  async close(tab: string): Promise<void> {
    this.leaving.add(tab)
    try {
      await nextTick()
      await this.api.close(tab)
      const list = this.list.value
      if (list) {
        this.list.value = { ...list, tabs: list.tabs.filter((item) => item.id !== tab) }
      }
    } finally {
      // A close the Host refused leaves a browser tab, which the next list adds back.
      this.leaving.delete(tab)
    }
  }

  /** A shown content watches its tab on the page's one view, opening the view when there is none. */
  show(tab: string): LiveSession {
    this.showing.add(tab)
    if (this.closing !== null) {
      clearTimeout(this.closing)
      this.closing = null
    }
    let session = this.session.value
    if (!session) {
      session = new LiveSession({
        open: this.api.stream,
        platform: viewerPlatform(navigator),
        onClipboard: (text) => viewerClipboard.receive(text),
        onOperation: this.api.onOperation,
        onTabs: (tabs) => this.adopt({ tabs }),
        onEnded: () => void this.refresh(),
      })
      this.session.value = session
      session.start()
    }
    session.watch(tab)
    return session
  }

  /**
   * A content that stops showing. Selecting another `browser` tab shows it in
   * the same flush, so the view closes only when none followed.
   */
  hide(tab: string): void {
    this.showing.delete(tab)
    if (this.showing.size > 0 || this.closing !== null) {
      return
    }
    this.closing = setTimeout(() => {
      this.closing = null
      if (this.showing.size === 0) {
        this.closeView()
      }
    }, 0)
  }

  private closeView(): void {
    this.session.value?.close()
    this.session.value = null
  }

  dispose(): void {
    this.disposed = true
    if (this.closing !== null) {
      clearTimeout(this.closing)
      this.closing = null
    }
    this.showing.clear()
    this.closeView()
  }
}

export function asTabsError(error: unknown): BrowserTabsError {
  if (error instanceof BrowserTabsError) {
    return error
  }
  return new BrowserTabsError('request_failed', error instanceof Error ? error.message : String(error))
}
