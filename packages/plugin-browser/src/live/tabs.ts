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
import type { BrowserTab, LiveDownload, LiveTab, NeededBrowser } from '../generated/plugin'
import { viewerClipboard } from './clipboard'
import { picturesSupported } from './pictures'
import type { HeadlineText, HostArtifact, OpenUserStream, PageContext, SentenceText } from '@demicodes/plugin-sdk'
import { LiveSession, type PanelReport } from './session'

/** What a new tab shows before the user goes anywhere. */
export const NEW_TAB_URL = 'about:blank'

/**
 * The id of the panel tab the plugin adds for the browser's tab `tab`, one
 * the agent or a page opened (`live-view.md` § A browser tab in the panel).
 */
export function addedPanelTab(tab: string): string {
  return `browser-${tab}`
}

/**
 * How long after the viewer's click or key in a tab a tab that page opens is
 * the viewer's, which a browser selects: the window in which the Host takes
 * text the page copies as the viewer's (`live-view.md` § Input).
 */
const OPENED_BY_USER_MS = 5000

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
  /**
   * The panel tab whose page opened it: by Open Link in New Tab, or by a link
   * or script of the page, which the plugin writes as it adds the tab
   * (`live-view.md` § A browser tab in the panel).
   */
  openedBy: z.string().optional(),
})
export type BrowserTabData = z.infer<typeof browserTabDataSchema>

export interface BrowserTabList {
  tabs: BrowserTab[]
  /** The browser a tab needs on the Host, as its installed artifacts name it. */
  browser: NeededBrowser
}

/** Why the strip cannot make a browser tab: the Host lacks the browser, which only the agent installs. */
export const NO_BROWSER: SentenceText = 'No browser on this Host. Ask the agent to run demi browser install.'

/**
 * A request the backend or the conversation browser refused, with the
 * answer's own code and message, or one that failed otherwise. A request
 * never fails for the page's connection: the shell's call waits for the
 * backend (`plugin-pages.md` § The page context).
 */
export class BrowserTabsError extends Error {
  constructor(
    /** The backend's code; null for a failure that is no answer of the backend's. */
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
  input_failed: 'The page didn’t receive your input.',
  capture_unavailable: 'This device can’t capture the browser’s pages.',
  capture_failed: 'The device couldn’t capture the page. It’s trying again.',
  capture_stopped: 'The device couldn’t capture the page.',
}

/** A refusal's code as a sentence the page shows (`live-view.md` § A browser tab in the panel). */
export function refusalSentence(code: string | null): SentenceText {
  return (code === null ? undefined : REFUSALS[code]) ?? 'The browser couldn’t do that.'
}

/**
 * The conversation browser's tab list and tab methods (`live-view.md` § The
 * tab methods) and its user stream, as the plugin's page context supplies
 * them. Every request rejects with a `BrowserTabsError`, except one the
 * panel session dropped as it ended, which rejects with the abort.
 */
export interface BrowserTabsApi {
  /** The tab list, as the plugin's conversation state follows it. */
  tabs: {
    /** The list last read; null before the first. */
    value: Readonly<Ref<BrowserTabList | null>>
    /** Why the last read failed, until one succeeds. */
    error: Readonly<Ref<BrowserTabsError | null>>
  }
  /**
   * Asks the plugin to open a browser tab for the panel tab, as Retry does;
   * answers the browser tab the panel tab shows now, or null when none.
   */
  bind(panelTab: string): Promise<string | null>
  /** Asks the plugin to read the browser's tabs and bring the panel's up to date. */
  sync(): Promise<void>
  /** Selects the panel tab `panelTab` and opens the panel, once the panel has it. */
  select(panelTab: string): void
  /** Starts loading `url`; answers the number of the last tab list before the request started. */
  navigate(tab: string, url: string): Promise<number>
  /** Moves or reloads the tab; answers as `navigate` does. */
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<number>
  /** Stops loading the tab's page; answers as `navigate` does. */
  stop(tab: string): Promise<number>
  stream: OpenUserStream
  /** What the Host holds of the browser's package, read reactively. */
  installed(): readonly HostArtifact[]
  /** Whether the Host is a Cloud that does not run yet, read reactively. */
  hostStarting(): boolean
}

/** Refusals of a Host that cannot be reached for the moment: an offline device, a stopped Cloud. */
const UNREACHED = new Set(['device_offline', 'host_stopped'])

/**
 * Whether a request failed because the backend could not reach the Host: it
 * changes no tab, since the browser may still run there, and is no defect of
 * the page (`live-view.md` § A browser tab in the panel).
 */
function unreached(error: unknown): boolean {
  return error instanceof BrowserTabsError && error.code !== null && UNREACHED.has(error.code)
}

/** Answers that will not change by asking again. */
const FINAL_CODES = new Set(['conversation_not_found', 'conversation_archived'])

/** Whether this web browser can show the view's pictures, once it has answered. */
export type PictureSupport = 'checking' | 'supported' | 'unsupported'

/**
 * What Demi does for a panel tab before the browser has its tab
 * (`live-view.md` § A browser tab in the panel): reads what the Host is
 * doing, starts the Cloud, starts the browser on the Host, or has the
 * browser open the tab.
 */
export type StartingPhase = 'connecting' | 'cloud' | 'browser' | 'page'

/** What the content says in each phase, from the frame the phase begins. */
export const STARTING_LABELS: Record<StartingPhase, SentenceText> = {
  connecting: 'Connecting…',
  cloud: 'Starting Cloud…',
  browser: 'Starting the browser…',
  page: 'Opening the page…',
}

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

/**
 * The plugin opening a browser tab for a panel tab again: asked, or answered
 * with the browser tab it opened, until the panel tab's data names that tab.
 */
type Binding =
  | { status: 'asked' }
  | { status: 'opened'; tab: string }

/** How the page reports failures: to the user in a toast, or a defect of the page to the console. */
export type PageErrors = PageContext['errors']

/** What a toast of a notice the picture still shows through names (`live-view.md` § Opening a view). */
const NOTICE_TITLE: HeadlineText = 'Could Not Operate the Browser'

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
  /** The panel tabs the plugin opens a browser tab for again, by id. */
  private readonly binding = shallowReactive(new Map<string, Binding>())
  /** The browser tabs the page showed a picture of, while the browser has them. */
  private readonly pictured = shallowReactive(new Set<string>())
  /** The downloads the user started in each browser tab, as a view last reported them, the newest last. */
  private readonly downloads = shallowReactive(new Map<string, readonly LiveDownload[]>())
  /** Every browser tab a view listed; null before the first list, whose tabs are not new to the page. */
  private heard: Set<string> | null = null
  /** The panel the shown tab's content measured, which a view sizes the tab by. */
  private panel: PanelReport | null = null
  /**
   * The panel tab whose content is shown, with its browser tab, which the
   * view watches while the page is visible; a content shown before the
   * browser has its tab keeps the view open watching nothing, so the Host
   * knows the panel's size for the tab it opens.
   */
  private readonly shownContent = shallowRef<{ panelTab: string; tab: string | null } | null>(null)
  /** The browser tab whose content is shown. */
  private readonly shownTab = computed(() => this.shownContent.value?.tab ?? null)
  /** The shown tab the plugin was asked to look for, until a view lists it again. */
  private missed: string | null = null
  private closing: ReturnType<typeof setTimeout> | null = null
  private readonly visibility: Readonly<Ref<DocumentVisibilityState>>
  private disposed = false
  /** Tells the view a panel that stopped resizing for a moment; after the view closed, nobody. */
  private readonly settled = useDebounceFn(() => this.session.value?.panel(), PANEL_SETTLE_MS)

  constructor(
    readonly api: BrowserTabsApi,
    private readonly errors: PageErrors,
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
    // A view the stopped Cloud refused connects at once once the Cloud runs, so the Host knows the panel's
    // size before it opens the tab a shown content waits for.
    watch(() => api.hostStarting(), (starting, was) => {
      if (was && !starting) {
        this.session.value?.reconnect()
      }
    })
    watch(this.listError, (error) => {
      if (error?.code && FINAL_CODES.has(error.code)) {
        this.closeView()
      }
    })
    const pictures = options.pictures ?? (() => picturesSupported(errors.defect))
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

  /** The downloads the user started in the browser tab `tab`, the newest last. */
  downloadsOf(tab: string | undefined): readonly LiveDownload[] {
    return tab === undefined ? [] : (this.downloads.get(tab) ?? [])
  }

  /**
   * Whether the panel tab bound to `tab`, asking for `url`, shows its page
   * loading (`live-view.md` § A browser tab in the panel). A request of the
   * user's loads from the moment it is made until a tab list numbered higher
   * than the one its answer names says otherwise, whichever reaches the page
   * first: on a far backend the browser may start loading well after the
   * answer, and a list numbered no higher describes the page as it was.
   * Otherwise the page loads while the browser last said it does: in a view,
   * or, for a tab not shown, in the plugin's tab list. A shown tab no view
   * reported yet is one whose browser tab is still opening, and it loads
   * while it asks for an address; one not shown that the list lacks too was
   * lost with the browser, and waits, not loading, until it is shown. A tab
   * shown again keeps what the browser last said, so a page that had loaded
   * shows no loading.
   */
  loading(tab: string | undefined, url: string): boolean {
    if (this.requested(tab)) {
      return true
    }
    const live = this.tab(tab)
    if (live) {
      return live.loading
    }
    // A tab the strip shows beside the shown one, which no view reported, as the plugin last listed it.
    if (tab !== undefined && tab !== this.shownTab.value) {
      return this.list.value?.tabs.find((candidate) => candidate.id === tab)?.loading ?? false
    }
    return url !== NEW_TAB_URL
  }

  /**
   * Whether a request of the user's on `tab` is under way: sent, or answered
   * while no tab list numbered after the answer reached the page. Until then
   * what the browser reports of the tab describes the page the request
   * leaves.
   */
  private requested(tab: string | undefined): boolean {
    const request = tab === undefined ? undefined : this.requests.get(tab)
    if (request?.status === 'asked') {
      return true
    }
    const listed = this.listed.value
    return request?.status === 'answered' && (listed === null || listed <= request.list)
  }

  /**
   * The title of the page the panel tab with `data` shows, as the browser
   * reports it: as a view last reported it, else as the plugin last listed
   * it, the moment the browser names it, as a web browser's tab takes its
   * page's title in the background too. None without a browser tab, for a
   * page without a title, or while the browser still reports the page a
   * request of the user's leaves: until a list after the request, and while
   * the browser loads from an address other than the one the tab asks for,
   * as it does until the new page commits (`live-view.md` § A browser tab in
   * the panel).
   */
  title(data: BrowserTabData): string | null {
    if (data.tab === undefined || data.closed || data.failure || this.requested(data.tab)) {
      return null
    }
    const reported = this.tab(data.tab) ?? this.list.value?.tabs.find((candidate) => candidate.id === data.tab)
    if (!reported || (reported.loading && reported.url !== data.url)) {
      return null
    }
    return reported.title || null
  }

  /**
   * Whether the panel tab `panelTab`, with `data`, shows its page loading:
   * the one state its strip's spinner, its Stop and its progress line show
   * (`live-view.md` § A browser tab in the panel). It loads while its browser
   * tab opens, the first time or again, and otherwise as its browser tab
   * does; and a shown tab that has no picture yet loads until its first
   * picture, unless none can come. A tab that could not open, or whose
   * browser tab the browser lost and no one opened again yet, does not, nor
   * does a new tab nobody sent anywhere yet, which loads nothing.
   */
  busy(panelTab: string, data: BrowserTabData): boolean {
    const blank = this.blank(data)
    if (this.opening(panelTab, data)) {
      return !blank
    }
    if (data.failure || data.closed) {
      return false
    }
    if (data.tab === undefined) {
      return !blank
    }
    return this.loading(data.tab, data.url) || (!blank && this.awaitsPicture(data.tab))
  }

  /**
   * Whether the panel tab with `data` is a new tab nobody sent anywhere yet:
   * its address is the new tab's and no request of the user's is under way on
   * its browser tab. It loads nothing, so it shows the blank New Tab, not
   * what Demi waits for, while the browser opens its tab unseen; only the
   * browser's own word that its page loads shows it loading
   * (`live-view.md` § A browser tab in the panel).
   */
  private blank(data: BrowserTabData): boolean {
    return data.url === NEW_TAB_URL && !this.requested(data.tab)
  }

  /**
   * Whether the browser tab `tab` is shown with no picture yet, which the
   * view will bring: not in a web browser that cannot show the pictures, nor
   * while the Host says it cannot capture the tab.
   */
  private awaitsPicture(tab: string): boolean {
    return tab === this.shownTab.value
      && !this.pictured.has(tab)
      && this.pictures.value !== 'unsupported'
      && !this.session.value?.state.pictureless
  }

  /**
   * What Demi does for the panel tab `panelTab`, with `data`, before the
   * browser has its tab (`live-view.md` § A browser tab in the panel): while
   * it opens, the first time or again, while it waits to, and while the
   * browser's tab list does not name the browser tab its data names, which
   * the browser may have lost, as after a restart. The Cloud starting comes
   * first, which the product state says; then connecting, while neither a
   * view nor the plugin's list has said what the conversation's browser
   * holds; then the browser starting, while it has no tabs; then the browser
   * opening the tab. Null once the list names the tab, for a tab that could
   * not open and was not retried, and for a new tab nobody sent anywhere yet,
   * which waits for nothing the user asked for.
   */
  startingPhase(panelTab: string, data: BrowserTabData): StartingPhase | null {
    if (this.blank(data)) {
      return null
    }
    const opening = this.opening(panelTab, data)
    if (!opening && data.failure) {
      return null
    }
    const tabs = this.browserTabs()
    const listed = data.tab !== undefined && !data.closed && tabs?.some((tab) => tab.id === data.tab) === true
    if (!opening && listed) {
      return null
    }
    if (this.api.hostStarting()) {
      return 'cloud'
    }
    if (tabs === null) {
      return 'connecting'
    }
    return tabs.length > 0 ? 'page' : 'browser'
  }

  /**
   * The conversation browser's tabs: as the view last reported them while it
   * is connected, else as the plugin last listed them; null while neither
   * has said what the browser holds.
   */
  private browserTabs(): readonly { id: string }[] | null {
    const view = this.session.value?.state
    if (view?.connection === 'live' || view?.connection === 'stalled') {
      return view.running ? view.tabs : []
    }
    return this.list.value?.tabs ?? null
  }

  /**
   * Whether the plugin opens a browser tab for the panel tab `panelTab`, with
   * `data`, again: from the ask until its data names the tab the plugin
   * answered, in whatever order the answer and the data reach the page.
   */
  opening(panelTab: string, data: BrowserTabData): boolean {
    const binding = this.binding.get(panelTab)
    if (!binding) {
      return false
    }
    return binding.status === 'asked' || data.tab !== binding.tab
  }

  /**
   * Asks the plugin to open a browser tab for the panel tab `panelTab` again:
   * Retry of a tab that could not open, and a tab shown after the browser
   * lost its browser tab. The tab loads from now on; a second ask while one
   * is on its way joins it. Rejects with what refused it.
   */
  async bind(panelTab: string): Promise<void> {
    if (this.binding.get(panelTab)?.status === 'asked') {
      return
    }
    const asked: Binding = { status: 'asked' }
    this.binding.set(panelTab, asked)
    let tab: string | null
    try {
      tab = await this.api.bind(panelTab)
    } catch (error) {
      this.binding.delete(panelTab)
      throw error
    }
    // None opened: the panel tab's data says why.
    if (tab === null) {
      this.binding.delete(panelTab)
      return
    }
    this.binding.set(panelTab, { status: 'opened', tab })
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
   * The tab menu's Reload of the panel tab with `data`, as the address bar's
   * Reload does it; a refusal is a toast. A tab without its browser tab has
   * nothing to reload.
   */
  reload(data: BrowserTabData): void {
    if (data.tab === undefined || data.closed) {
      return
    }
    this.history(data.tab, 'reload').catch((error: unknown) => this.errors.report('Could Not Reload the Page', error))
  }

  /**
   * The user's Stop on `tab`. The tab loads until a tab list numbered after
   * the answer says it stopped, as after any request; rejects with what
   * refused it.
   */
  stop(tab: string): Promise<void> {
    return this.request(tab, () => this.api.stop(tab))
  }

  /**
   * Runs a request of the user's on `tab`. The latest one on the tab is the
   * one its loading follows; a refusal ends it at once, and the caller
   * reports it. A tab the browser no longer has was lost with the browser,
   * as when Demi restarted: nothing says so, and the plugin is asked to read
   * the list, which marks the panel tab lost, so it opens again on its
   * address, at once while it is shown, as a browser reloads a discarded tab
   * (`live-view.md` § A browser tab in the panel).
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
      if (error instanceof BrowserTabsError && error.code === 'tab_not_found') {
        this.lookFor(tab)
        return
      }
      throw error
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
   * when there is none; one shown before the browser has its tab watches
   * nothing, and the view tells the Host the panel's size meanwhile. A view
   * waiting to reconnect connects at once for a tab it did not show, as a tab
   * that just got its browser tab: the view's end may have been only that
   * there was no browser yet. A hidden page
   * opens none until it is shown, and a web browser that cannot show the
   * pictures none at all.
   */
  show(panelTab: string, tab: string | null): void {
    const another = this.shownTab.value !== tab
    this.shownContent.value = { panelTab, tab }
    if (this.closing !== null) {
      clearTimeout(this.closing)
      this.closing = null
    }
    if (another && tab !== null) {
      this.session.value?.reconnect()
    }
    this.watchShown()
  }

  /**
   * The content of `panelTab` stops showing. Selecting another `browser` tab
   * shows it in the same flush, before or after this, so the view closes
   * only when none followed.
   */
  hide(panelTab: string): void {
    if (this.shownContent.value?.panelTab !== panelTab) {
      return
    }
    this.shownContent.value = null
    if (this.closing !== null) {
      return
    }
    this.closing = setTimeout(() => {
      this.closing = null
      if (this.shownContent.value === null) {
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
      this.shownContent.value === null
      || this.panel === null
      || this.visibility.value !== 'visible'
      || this.pictures.value !== 'supported'
    ) {
      return
    }
    this.view().watch(this.shownTab.value)
  }

  /**
   * The view's tab list. Each tab is kept as reported, and one the browser no
   * longer has leaves. A list without the shown tab asks the plugin, once
   * for that tab, to read the browser's tabs: the plugin marks the panel tab
   * closed if the browser lost it.
   */
  private viewTabs(tabs: readonly LiveTab[], list: number): void {
    this.listed.value = list
    this.pageOpened(tabs)
    for (const id of [...this.known.keys()]) {
      if (!tabs.some((tab) => tab.id === id)) {
        this.known.delete(id)
        this.pictured.delete(id)
      }
    }
    for (const tab of tabs) {
      this.known.set(tab.id, tab)
    }
    const shown = this.shownTab.value
    if (shown === null) {
      return
    }
    // A list with the shown tab answers the last look: a later loss is looked for again.
    if (tabs.some((tab) => tab.id === shown)) {
      this.missed = null
      return
    }
    this.lookForShown()
  }

  /**
   * A tab a page opened appears in the strip at once, as in any browser
   * (`live-view.md` § A browser tab in the panel): the plugin adds it when
   * asked to read the browser's tabs. One that the page the user watches
   * opened right after the user's click or key there is the user's, and is
   * selected, as a browser selects the tab a click opens; one a page opened
   * by itself is only added. A read that cannot reach the Host is asked for
   * again with the next list.
   */
  private pageOpened(tabs: readonly LiveTab[]): void {
    const heard = this.heard
    this.heard = new Set([...(heard ?? []), ...tabs.map((tab) => tab.id)])
    if (heard === null) {
      return
    }
    const opened = tabs.filter((tab) => tab.createdBy.kind === 'page' && !heard.has(tab.id))
    if (opened.length === 0) {
      return
    }
    this.api.sync().catch((error: unknown) => {
      if (!unreached(error)) {
        this.errors.defect('The panel could not add the tabs a page opened', error)
        return
      }
      for (const tab of opened) {
        this.heard?.delete(tab.id)
      }
    })
    const shown = this.shownTab.value
    const view = this.session.value
    for (const tab of opened) {
      const opener = tab.createdBy.kind === 'page' ? tab.createdBy.opener : null
      if (opener !== null && opener === shown && view?.pressedLately(opener, OPENED_BY_USER_MS)) {
        this.api.select(addedPanelTab(tab.id))
      }
    }
  }

  /** Asks the plugin to read the browser's tabs for `tab`, which the browser no longer has. */
  private lookFor(tab: string): void {
    if (tab === this.shownTab.value) {
      // Asked again even when a look for it is under way: that one may have
      // read the list before the browser lost the tab.
      this.missed = null
      this.lookForShown()
      return
    }
    this.api.sync().catch((error: unknown) => {
      if (!unreached(error)) {
        this.errors.defect('The panel could not look for a lost tab', error)
      }
    })
  }

  /**
   * Asks the plugin, once for the shown tab, to read the browser's tabs: the
   * plugin removes the panel tab if its browser tab was closed, or marks it
   * lost, which opens it again at once while it is shown. A view that ends
   * asks too, since a browser that ended or a Cloud that stopped took the
   * tab with it, and the view may wait long before it reads a list again.
   * A read that cannot reach the Host changes no tab, and the next list or
   * end asks again.
   */
  private lookForShown(): void {
    const shown = this.shownTab.value
    if (shown === null || this.missed === shown) {
      return
    }
    this.missed = shown
    this.api.sync().catch((error: unknown) => {
      if (!unreached(error)) {
        this.errors.defect('The panel could not look for a closed tab', error)
        return
      }
      if (this.missed === shown) {
        this.missed = null
      }
    })
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
      onNotice: (code) => this.notice(code),
      onEnded: () => this.lookForShown(),
      onPicture: (tab) => this.pictured.add(tab),
      onDownloads: (tab, downloads) => this.downloads.set(tab, downloads),
      defect: this.errors.defect,
    })
    this.session.value = session
    session.start()
    return session
  }

  /** A notice the picture still shows through is a failed request: a toast in the Writing page's words for its code. */
  private notice(code: string): void {
    this.errors.report(NOTICE_TITLE, new BrowserTabsError(code, refusalSentence(code)))
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
    this.shownContent.value = null
    this.known.clear()
    this.requests.clear()
    this.binding.clear()
    this.pictured.clear()
    this.downloads.clear()
    this.closeView()
  }
}

export function asTabsError(error: unknown): BrowserTabsError {
  if (error instanceof BrowserTabsError) {
    return error
  }
  return new BrowserTabsError(null, error instanceof Error ? error.message : String(error))
}
