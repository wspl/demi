/**
 * The `browser` page's panel session (`live-view.md` § A browser tab in the
 * panel): what a tab saves, the conversation browser's tab list, which the
 * plugin's conversation state brings, the requests that open, close and
 * navigate its tabs, and the one view a page keeps while a `browser` tab is
 * shown and the page is visible.
 */
import { useDocumentVisibility } from '@vueuse/core'
import { computed, nextTick, shallowRef, watch, type ComputedRef, type Ref, type ShallowRef } from 'vue'
import { z } from 'zod'
import type { BrowserCreatedBy } from '../generated/plugin'
import { viewerClipboard } from './clipboard'
import { viewerPlatform } from './input'
import { picturesSupported } from './pictures'
import type { HostInstall } from '@demicodes/plugin-sdk'
import type { OpenUserStream } from '@demicodes/plugin-sdk'
import { LiveSession } from './session'

/** What a new tab shows before the user goes anywhere. */
export const NEW_TAB_URL = 'about:blank'

export const browserTabDataSchema = z.object({
  /** The address the tab shows, saved as the page changes it. */
  url: z.string(),
  /** The conversation browser's tab this panel tab is bound to, once it has one. */
  tab: z.string().optional(),
})
export type BrowserTabData = z.infer<typeof browserTabDataSchema>

/** A tab of the conversation's browser, as its tab list names it. */
export interface BrowserTabInfo {
  id: string
  title: string
  url: string
  createdBy: BrowserCreatedBy
}

export interface BrowserTabList {
  tabs: BrowserTabInfo[]
}

/** A request the backend or the conversation browser refused, with the answer's own code and message. */
export class BrowserTabsError extends Error {
  constructor(
    /** The backend's code; null when something in front of it answered. */
    readonly code: string | null,
    message: string,
  ) {
    super(message)
  }
}

/**
 * The conversation browser's tab list and tab methods (`live-view.md` § The
 * tab methods) and its user stream, as the plugin's page context supplies
 * them. Every request rejects with a `BrowserTabsError`.
 */
export interface BrowserTabsApi {
  /** The tab list, as the plugin's conversation state follows it. */
  tabs: {
    /** The list last read; null before the first. */
    value: Readonly<Ref<BrowserTabList | null>>
    /** Why the last read failed, until one succeeds. */
    error: Readonly<Ref<BrowserTabsError | null>>
    /** Reads the list again now. */
    read(): void
  }
  open(url: string): Promise<BrowserTabInfo>
  close(tab: string): Promise<void>
  navigate(tab: string, url: string): Promise<void>
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<void>
  stream: OpenUserStream
  /** What the Host installs before the browser can start, read reactively. */
  installs(): readonly HostInstall[]
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

/** Whether this web browser can show the view's pictures, once it has answered. */
export type PictureSupport = 'checking' | 'supported' | 'unsupported'

export interface BrowserTabsOptions {
  /** The page's visibility; the document's own unless a test supplies one. */
  visibility?: Readonly<Ref<DocumentVisibilityState>>
  /** Resolves whether this web browser can decode the view's pictures; WebCodecs' answer unless the gallery or a test supplies one. */
  pictures?: () => Promise<boolean>
}

/** Reports a defect of the page itself, as the page context's `errors.defect` does. */
export type ReportDefect = (message: string, error: unknown) => void

/** Whether the page is visible: one listener, for the page's lifetime, that every controller shares. */
const pageVisibility = useDocumentVisibility()

/**
 * One conversation's browser, for one page, made by the page's panel
 * session in its effect scope: the tab list it last learned, and the view,
 * open only while a `browser` tab's content is shown and the page is
 * visible (`live-view.md` § Ending a view).
 */
export class BrowserTabsController {
  /** The last list the plugin's state or the view gave; null before the first. */
  readonly list: ShallowRef<BrowserTabList | null> = shallowRef(null)
  /** Why the last list could not be read, until one is. */
  readonly listError: ComputedRef<BrowserTabsError | null>
  readonly session: ShallowRef<LiveSession | null> = shallowRef(null)
  /**
   * Whether this web browser can show the pictures, asked once. One that
   * cannot opens no view, so it is no viewer (`live-view.md` § A browser tab
   * in the panel).
   */
  readonly pictures: ShallowRef<PictureSupport> = shallowRef('checking')
  /**
   * The browser tab this page opened for each panel tab. A panel tab that
   * lost its binding, to a stale read of the saved panel for one, gets its own
   * tab back instead of a second one.
   */
  private readonly opened = new Map<string, string>()
  /** Opens in flight by panel tab, so a content shown twice asks once. */
  private readonly opening = new Map<string, Promise<BrowserTabInfo>>()
  /** Browser tabs whose panel tab the user closed, until the browser has closed them too. */
  private readonly leaving = new Set<string>()
  /** The browser tab whose content is shown, which the view watches while the page is visible. */
  private shown: string | null = null
  private closing: ReturnType<typeof setTimeout> | null = null
  private readonly visibility: Readonly<Ref<DocumentVisibilityState>>
  private disposed = false

  constructor(
    readonly api: BrowserTabsApi,
    private readonly panel: BrowserPanelTabs,
    private readonly defect: ReportDefect,
    options: BrowserTabsOptions = {},
  ) {
    this.visibility = options.visibility ?? pageVisibility
    this.listError = computed(() => api.tabs.error.value)
    // The watchers stop with the panel session's effect scope.
    watch(this.visibility, (state) => this.visibilityChanged(state), { flush: 'sync' })
    watch(
      api.tabs.value,
      (list) => {
        if (list) {
          this.adopt(list)
        }
      },
      { immediate: true, flush: 'sync' },
    )
    watch(this.listError, (error) => {
      if (error?.code && FINAL_CODES.has(error.code)) {
        this.closeView()
      }
    })
    const pictures = options.pictures ?? (() => picturesSupported(defect))
    void pictures().then((supported) => {
      if (this.disposed) {
        return
      }
      this.pictures.value = supported ? 'supported' : 'unsupported'
      this.watchShown()
    })
  }

  /** Reads the tab list again, as the content's Retry does. */
  refresh(): void {
    this.api.tabs.read()
  }

  /**
   * A list from the plugin's state or from the view's `state`, whose panel
   * tabs it adds; nothing is ever removed here: a tab the Host lost stays the
   * user's to close. While a tab is being opened the list may already name
   * it, before its panel tab is bound: it would be taken for the agent's.
   * Adding waits for the open to settle.
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
    const mine = this.opened.get(panelTabId)
    const existing = this.list.value?.tabs.find((tab) => tab.id === mine)
    if (existing) {
      bind(existing)
      return Promise.resolve(existing)
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
        this.opened.set(panelTabId, tab.id)
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

  /**
   * A shown content watches its tab on the page's one view, opening the view
   * when there is none. A hidden page opens none until it is shown, and a
   * web browser that cannot show the pictures none at all.
   */
  show(tab: string): void {
    this.shown = tab
    if (this.closing !== null) {
      clearTimeout(this.closing)
      this.closing = null
    }
    this.watchShown()
  }

  /**
   * A content that stops showing. Selecting another `browser` tab shows it in
   * the same flush, before or after this, so the view closes only when none
   * followed.
   */
  hide(tab: string): void {
    if (this.shown !== tab) {
      return
    }
    this.shown = null
    if (this.closing !== null) {
      return
    }
    this.closing = setTimeout(() => {
      this.closing = null
      if (this.shown === null) {
        this.closeView()
      }
    }, 0)
  }

  /**
   * Nobody can watch a hidden page, and an open view keeps its conversation
   * active, so the view closes while the page is hidden. Shown again, the
   * page opens a new view on the shown tab; the tab list reached it
   * meanwhile with the plugin's state.
   */
  private visibilityChanged(state: DocumentVisibilityState): void {
    if (state !== 'visible') {
      this.closeView()
      return
    }
    this.watchShown()
  }

  /** The shown tab on the page's one view, while someone can see its pictures. */
  private watchShown(): void {
    if (this.shown === null || this.visibility.value !== 'visible' || this.pictures.value !== 'supported') {
      return
    }
    this.view().watch(this.shown)
  }

  /** The page's one view, opened when there is none. */
  private view(): LiveSession {
    const open = this.session.value
    if (open) {
      return open
    }
    const session = new LiveSession({
      open: this.api.stream,
      platform: viewerPlatform(navigator),
      onClipboard: (text) => viewerClipboard.receive(text),
      onTabs: (tabs) => this.adopt({ tabs }),
      // The browser may have ended with the view, which no job told the plugin.
      onEnded: () => this.refresh(),
      defect: this.defect,
    })
    this.session.value = session
    session.start()
    return session
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
    this.shown = null
    this.closeView()
  }
}

export function asTabsError(error: unknown): BrowserTabsError {
  if (error instanceof BrowserTabsError) {
    return error
  }
  return new BrowserTabsError(null, error instanceof Error ? error.message : String(error))
}
