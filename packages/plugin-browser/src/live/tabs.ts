/**
 * The `browser` page's panel session (`live-view.md` § A browser tab in the
 * panel): what a tab keeps, the conversation browser's tab list, which the
 * plugin's conversation state brings, the requests that move a bound tab
 * and the user's latest one on each, the panel's size, each browser tab as a
 * view last reported it, and the one view a page keeps while a `browser` tab
 * is shown and the page is visible. Which browser tab a panel tab shows is
 * the plugin's work on the backend; the session never opens, closes or adds
 * a tab.
 */
import { clientPlatform } from '@demicodes/utils'
import { useDocumentVisibility, useDebounceFn } from '@vueuse/core'
import { computed, shallowReactive, shallowRef, watch, type ComputedRef, type Ref, type ShallowRef } from 'vue'
import { z } from 'zod'
import type { BrowserTab, LiveTab, NeededBrowser } from '../generated/plugin'
import { viewerClipboard } from './clipboard'
import { picturesSupported } from './pictures'
import type { HostArtifact, OpenUserStream, SentenceText } from '@demicodes/plugin-sdk'
import { LiveSession, type PanelReport } from './session'

/** What a new tab shows before the user goes anywhere. */
export const NEW_TAB_URL = 'about:blank'

export const browserTabDataSchema = z.object({
  /** The address the tab shows, or the one its user last asked for while it has no browser tab. */
  url: z.string(),
  /** The conversation browser's tab it shows, once the plugin opened one. */
  tab: z.string().optional(),
  /** The browser no longer has that tab. */
  closed: z.boolean().optional(),
  /** Why the plugin could not open a browser tab for it. */
  failure: z.object({ code: z.string(), message: z.string() }).optional(),
  /** The page's title as the address bar last showed it, so the strip names the tab while no view shows it. */
  title: z.string().optional(),
  /** How many times the agent showed the browser tab; absent while it never did (`live-view.md` § Showing a tab). */
  shows: z.number().int().min(1).optional(),
})
export type BrowserTabData = z.infer<typeof browserTabDataSchema>

export interface BrowserTabList {
  tabs: BrowserTab[]
  /** The browser a tab needs on the Host, as its installed artifacts name it. */
  browser: NeededBrowser
}

/** Why the strip cannot make a browser tab: the Host lacks the browser, which only the agent installs. */
export const NO_BROWSER: SentenceText = 'No browser on this Host. Ask the agent to run demi browser install.'

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

/** What a refusal's code means to the user, in the Writing page's words rather than the Host's error text. */
const REFUSALS: Readonly<Record<string, SentenceText>> = {
  history_boundary: 'The tab has no page to go to in that direction.',
  invalid_input: 'The browser opens only web, file and blank pages.',
  tab_not_found: 'This page is no longer open on the device.',
  host_stopped: 'The Cloud is stopped.',
  device_offline: 'The device is offline.',
  conversation_busy: 'The conversation is busy. Try again in a moment.',
  conversation_archived: 'This conversation is archived.',
  navigation_failed: 'The browser couldn’t load the page.',
  timeout: 'The browser didn’t answer in time.',
  browser_unavailable: 'The browser on the device isn’t running.',
  browser_lost: 'The browser on the device isn’t running.',
}

/** A refusal's code as a sentence the page shows (`live-view.md` § A browser tab in the panel). */
export function refusalSentence(code: string | null): SentenceText {
  return (code === null ? undefined : REFUSALS[code]) ?? 'The browser couldn’t do that.'
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
  }
  /** Asks the plugin to open a browser tab for the panel tab, as Retry and Reload do. */
  bind(panelTab: string): Promise<void>
  /** Asks the plugin to read the browser's tabs and bring the panel's up to date. */
  sync(): Promise<void>
  /** Starts loading `url`; answers the number of the last tab list before the request started. */
  navigate(tab: string, url: string): Promise<number>
  /** Moves or reloads the tab; answers as `navigate` does. */
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<number>
  stream: OpenUserStream
  /** What the Host holds of the browser's package, read reactively. */
  installed(): readonly HostArtifact[]
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

/**
 * The user's latest request on a browser tab (`live-view.md` § A browser tab
 * in the panel): sent and not answered yet, or answered with the number of
 * the last tab list before it started, which only a list numbered higher
 * outdates. A refused request leaves none.
 */
type TabRequest =
  | { status: 'asked' }
  | { status: 'answered'; list: number }

/** Reports a defect of the page itself, as the page context's `errors.defect` does. */
export type ReportDefect = (message: string, error: unknown) => void

/** Whether the page is visible: one listener, for the page's lifetime, that every controller shares. */
const pageVisibility = useDocumentVisibility()

/** How long a panel that resizes waits before the module hears of it: a drag would otherwise resize the capture each step. */
const PANEL_SETTLE_MS = 100

/**
 * One conversation's browser, for one page, made by the page's panel
 * session in its effect scope: the tab list, each browser tab as a view last
 * reported it, the panel the shown tab is sized by, and the view, open only
 * while a `browser` tab's content is shown and the page is visible
 * (`live-view.md` § Ending a view). It alone tells the view which tab to
 * watch.
 */
export class BrowserTabsController {
  /** The last list the plugin's state gave; null before the first. */
  readonly list: ComputedRef<BrowserTabList | null>
  /** Why the last list could not be read, until one is. */
  readonly listError: ComputedRef<BrowserTabsError | null>
  /**
   * Why a new browser tab cannot be made: the Host lacks the browser the
   * list names (`live-view.md` § A browser tab in the panel). Null while it
   * has it, and until the first list says which browser that is.
   */
  readonly unavailable: ComputedRef<SentenceText | null>
  readonly session: ShallowRef<LiveSession | null> = shallowRef(null)
  /**
   * Whether this web browser can show the pictures, asked once. One that
   * cannot opens no view, so it is no viewer (`live-view.md` § A browser tab
   * in the panel).
   */
  readonly pictures: ShallowRef<PictureSupport> = shallowRef('checking')
  /**
   * Each browser tab as a view last reported it, kept while no view is open,
   * so a tab shown again shows its address, loading and viewport at once.
   * A tab leaves once a view reports the browser without it.
   */
  private readonly known = shallowReactive(new Map<string, LiveTab>())
  /** The number of the last tab list a view reported, in the Host's sequence; null before the first. */
  private readonly listed = shallowRef<number | null>(null)
  /** The user's latest request on each browser tab. */
  private readonly requests = shallowReactive(new Map<string, TabRequest>())
  /** The panel the shown tab's content measured, which a view sizes the tab by. */
  private panel: PanelReport | null = null
  /** The browser tab whose content is shown, which the view watches while the page is visible. */
  private shown: string | null = null
  /** The shown tab the view found gone, which the plugin was asked to look for once. */
  private missed: string | null = null
  private closing: ReturnType<typeof setTimeout> | null = null
  private readonly visibility: Readonly<Ref<DocumentVisibilityState>>
  private disposed = false
  /** Tells the view a panel that stopped resizing for a moment; after the view closed, nobody. */
  private readonly settled = useDebounceFn(() => this.session.value?.panel(), PANEL_SETTLE_MS)

  constructor(
    readonly api: BrowserTabsApi,
    private readonly defect: ReportDefect,
    options: BrowserTabsOptions = {},
  ) {
    this.visibility = options.visibility ?? pageVisibility
    this.list = computed(() => api.tabs.value.value)
    this.listError = computed(() => api.tabs.error.value)
    this.unavailable = computed(() => {
      const needed = this.list.value?.browser
      if (!needed) {
        return null
      }
      const held = api
        .installed()
        .some((artifact) => artifact.name === needed.name && artifact.version === needed.version)
      return held ? null : NO_BROWSER
    })
    // The watchers stop with the panel session's effect scope.
    watch(this.visibility, (state) => this.visibilityChanged(state), { flush: 'sync' })
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

  /** The browser tab `tab` as a view last reported it, if one did. */
  tab(tab: string | undefined): LiveTab | null {
    return tab === undefined ? null : (this.known.get(tab) ?? null)
  }

  /**
   * Whether the panel tab bound to `tab`, asking for `url`, shows its page
   * loading (`live-view.md` § A browser tab in the panel). A request of the
   * user's loads from the moment it is made until a tab list numbered higher
   * than the one its answer names says otherwise, whichever reaches the page
   * first: on a far backend the browser may start loading well after the
   * answer, and a list numbered no higher describes the page as it was.
   * Otherwise the page loads while the browser last said it does, or, before
   * the browser ever reported the tab, as for one whose browser tab is still
   * opening, while the tab asks for an address. A tab shown again keeps what
   * the browser last said, so a page that had loaded shows no loading.
   */
  loading(tab: string | undefined, url: string): boolean {
    const request = tab === undefined ? undefined : this.requests.get(tab)
    if (request?.status === 'asked') {
      return true
    }
    const listed = this.listed.value
    if (request?.status === 'answered' && (listed === null || listed <= request.list)) {
      return true
    }
    const live = this.tab(tab)
    return live ? live.loading : url !== NEW_TAB_URL
  }

  /** The user's address, loaded in `tab`; rejects with what refused it. */
  navigate(tab: string, url: string): Promise<void> {
    return this.request(tab, () => this.api.navigate(tab, url))
  }

  /** The user's Back, Forward or Reload on `tab`; rejects with what refused it. */
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<void> {
    return this.request(tab, () => this.api.history(tab, action))
  }

  /**
   * Runs a request of the user's on `tab`. The latest one on the tab is the
   * one its loading follows; a refusal ends it at once, and the caller
   * reports it.
   */
  private async request(tab: string, run: () => Promise<number>): Promise<void> {
    const asked: TabRequest = { status: 'asked' }
    this.requests.set(tab, asked)
    let list: number
    try {
      list = await run()
    } catch (error) {
      if (this.requests.get(tab) === asked) {
        this.requests.delete(tab)
      }
      throw asTabsError(error)
    }
    // A later request on the tab replaced this one.
    if (this.requests.get(tab) === asked) {
      this.requests.set(tab, { status: 'answered', list })
    }
  }

  /**
   * The shown content measured its panel. The first measure goes to the view
   * at once, as a capture starts at it; later ones once the panel stopped
   * resizing for a moment.
   */
  resize(panel: PanelReport): void {
    const first = this.panel === null
    this.panel = panel
    if (first) {
      this.watchShown()
      return
    }
    void this.settled()
  }

  /**
   * A shown content watches its tab on the page's one view, opening the view
   * when there is none. A view waiting to reconnect connects at once for a
   * tab it did not show, as a tab that just got its browser tab: the view's
   * end may have been only that there was no browser yet. A hidden page
   * opens none until it is shown, and a web browser that cannot show the
   * pictures none at all.
   */
  show(tab: string): void {
    const another = this.shown !== tab
    this.shown = tab
    if (this.closing !== null) {
      clearTimeout(this.closing)
      this.closing = null
    }
    if (another) {
      this.session.value?.reconnect()
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
   * page opens a new view on the shown tab.
   */
  private visibilityChanged(state: DocumentVisibilityState): void {
    if (state !== 'visible') {
      this.closeView()
      return
    }
    this.watchShown()
  }

  /**
   * The shown tab on the page's one view, while someone can see its
   * pictures, once its content measured the panel the tab is sized by.
   */
  private watchShown(): void {
    if (
      this.shown === null
      || this.panel === null
      || this.visibility.value !== 'visible'
      || this.pictures.value !== 'supported'
    ) {
      return
    }
    this.view().watch(this.shown)
  }

  /**
   * The view's tab list. Each tab is kept as reported, and one the browser no
   * longer has leaves. A list without the shown tab asks the plugin, once
   * for that tab, to read the browser's tabs: the plugin marks the panel tab
   * closed if the browser lost it.
   */
  private viewTabs(tabs: readonly LiveTab[], list: number): void {
    this.listed.value = list
    for (const id of [...this.known.keys()]) {
      if (!tabs.some((tab) => tab.id === id)) {
        this.known.delete(id)
      }
    }
    for (const tab of tabs) {
      this.known.set(tab.id, tab)
    }
    const shown = this.shown
    if (shown === null || tabs.some((tab) => tab.id === shown) || this.missed === shown) {
      return
    }
    this.missed = shown
    this.api.sync().catch((error: unknown) => this.defect('The panel could not look for a closed tab', error))
  }

  /** The page's one view, opened when there is none. */
  private view(): LiveSession {
    const open = this.session.value
    if (open) {
      return open
    }
    const session = new LiveSession({
      open: this.api.stream,
      panel: () => this.panel,
      platform: clientPlatform(navigator),
      onClipboard: (text) => viewerClipboard.receive(text),
      onTabs: (tabs, list) => this.viewTabs(tabs, list),
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
    this.known.clear()
    this.requests.clear()
    this.closeView()
  }
}

export function asTabsError(error: unknown): BrowserTabsError {
  if (error instanceof BrowserTabsError) {
    return error
  }
  return new BrowserTabsError(null, error instanceof Error ? error.message : String(error))
}
