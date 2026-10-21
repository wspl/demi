/**
 * One conversation's browser on the page, which the plugin's store keeps
 * while the plugin is on (`../index.ts`). It holds:
 *
 * - what a tab keeps;
 * - the conversation browser's tab list, which the plugin's conversation state
 *   brings;
 * - the requests that move a bound tab, and the user's latest one on each;
 * - the panel's size;
 * - each browser tab as a view last reported it, and its last picture;
 * - the one view a page keeps, watched while a `browser` tab is shown and the
 *   page is visible, and kept unwatched otherwise.
 *
 * Which browser tab a panel tab shows is the plugin's work on the backend; the
 * session never opens, closes or adds a tab.
 *
 * Why it is built so:
 *
 * - The browser has a tab only once its tab list names it, since a panel tab
 *   may name one the browser lost.
 * - A request shows its loading before it leaves the page and ends only on a
 *   tab list numbered after it (the Host's numbers only grow), whatever order
 *   answer and lists arrive in.
 * - The view stays open while a shown tab opens, so the Host learns the panel's
 *   size before it creates the tab.
 * - A page that cannot decode H.264 opens no view, since it would lower the
 *   picture for every viewer with missing acknowledgements and decide the tab's
 *   size with blind clicks.
 * - A view nobody watches, as when another tab is selected, the panel closes,
 *   the conversation changes or the page hides, stays open watching no tab:
 *   the Host stops capturing, the stream says it is unwatched so nothing stays
 *   awake for it, and shown again it watches at once on the same connection,
 *   over the tab's last picture.
 */
import { clientPlatform } from '@demicodes/utils'
import { useDocumentVisibility, useDebounceFn } from '@vueuse/core'
import { computed, shallowReactive, shallowRef, watch, type ComputedRef, type Ref, type ShallowRef } from 'vue'
import { z } from 'zod'
import { panelReasonSchema, type BrowserTabs, type LiveDownload, type LiveTab, type PanelReason } from '../generated/plugin'
import { viewerClipboard } from './clipboard'
import { picturesSupported } from '@demicodes/plugin-sdk'
import type { OpenUserStream, PageContext, SentenceText } from '@demicodes/plugin-sdk'
import { LiveSession, type LiveStream, type PanelReport } from './session'
import { t } from '../locales/text'

/**
 * The address of a blank page: what a tab shows once its user stopped it
 * before the browser had it.
 */
export const BLANK_URL = 'about:blank'

/**
 * The id of the panel tab the plugin adds for the browser's tab `tab`, one
 * the agent or a page opened.
 */
export function addedPanelTab(tab: string): string {
  return `browser-${tab}`
}

/**
 * How long after the viewer's click or key in a tab a tab that page opens is
 * the viewer's, which a browser selects: the window in which the Host takes
 * text the page copies as the viewer's.
 */
const OPENED_BY_USER_MS = 5000

export const browserTabDataSchema = z.object({
  /** The address the tab shows, or the one its user last asked for while the browser does not have it. */
  url: z.string(),
  /** The conversation browser's tab it shows, once the plugin opened one. */
  tab: z.string().optional(),
  /** The browser no longer has that tab. */
  closed: z.boolean().optional(),
  /**
   * Why the plugin could not open a browser tab for it: the reason, or the
   * HTTP status a call would have answered the same failure with, and the
   * Host's or the plugin's message, which the page never shows.
   */
  failure: z.union([
    z.object({ code: panelReasonSchema, message: z.string() }),
    z.object({ status: z.string(), message: z.string() }),
  ]).optional(),
  /** The page's title as the address bar last showed it, so the strip names the tab while no view shows it. */
  title: z.string().optional(),
  /** How many times the agent showed the browser tab; absent while it never did. */
  shows: z.number().int().min(1).optional(),
  /**
   * The panel tab whose page opened it, by a link or script of the page,
   * which the plugin writes as it adds the tab.
   */
  openedBy: z.string().optional(),
})
export type BrowserTabData = z.infer<typeof browserTabDataSchema>

/** The plugin's conversation state: the conversation browser's tabs. */
export type BrowserTabList = BrowserTabs

/** Why the plugin could not open a browser tab for a panel tab, as its data keeps it. */
export type TabFailure = NonNullable<BrowserTabData['failure']>

/**
 * What every reason the page can be told means to the user, in the Writing
 * page's words rather than the Host's error text: the browser's own codes, the
 * view's capture notices, the conversation's host access's refusals and the
 * plugin call route's own. The catalogue's `reasons` hold a sentence for each,
 * so a reason without one does not build.
 */
export function reasonSentence(reason: PanelReason): SentenceText {
  return t(`reasons.${reason}`)
}

/** What a tab the plugin could not open says, from its data. */
export function failureSentence(failure: TabFailure): SentenceText {
  return 'code' in failure ? reasonSentence(failure.code) : serverRefused(failure.status)
}

/** What a failure with no reason says when the backend answered with `status`, such as `500 Internal Server Error`. */
export function serverRefused(status: string): SentenceText {
  return t('answers.refused', { status })
}

/**
 * What a request failed for: a reason of the browser's or the backend's, the
 * status of an answer without one, or that no answer came (a network
 * failure, a timeout) or that the answer could not be read.
 */
export type TabsFailure =
  | { reason: PanelReason }
  | { status: string }
  | { answer: 'none' | 'unreadable' }

/** The page's words for a failure, in the page's language as of now. */
function tabsFailureSentence(failure: TabsFailure): SentenceText {
  if ('reason' in failure) {
    return reasonSentence(failure.reason)
  }
  return 'status' in failure ? serverRefused(failure.status) : t(`answers.${failure.answer}`)
}

/**
 * A request that failed, keeping what it failed for. Its `message` is worded as
 * it is read, from the failure, so a language chosen while it shows rewords it,
 * as a `ShownError` does. A request never fails for the page's connection: the
 * shell's call waits for the backend (`packages/web-ui/src/plugins/page.ts`).
 */
export class BrowserTabsError extends Error {
  /** The backend's or the browser's reason; null for a failure that carries none. */
  readonly reason: PanelReason | null

  constructor(readonly failure: TabsFailure) {
    super()
    this.reason = 'reason' in failure ? failure.reason : null
    // `message` is a property of Error, which an accessor cannot override in a subclass.
    Object.defineProperty(this, 'message', { get: () => tabsFailureSentence(this.failure), configurable: true })
  }

  static of(reason: PanelReason): BrowserTabsError {
    return new BrowserTabsError({ reason })
  }
}

/**
 * The conversation browser's tab list and tab methods and its user stream, as
 * the plugin's page context supplies them. Every request rejects with a
 * `BrowserTabsError`, except one dropped as its browser was released, which
 * rejects with the abort.
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
  /** Selects the panel tab `panelTab` and opens the panel, once the panel has it. */
  select(panelTab: string): void
  /** Starts loading `url`; answers the number of the last tab list before the request started. */
  navigate(tab: string, url: string): Promise<number>
  /** Moves or reloads the tab; answers as `navigate` does. */
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<number>
  /** Stops loading the tab's page; answers as `navigate` does. */
  stop(tab: string): Promise<number>
  stream: OpenUserStream
  /** Whether the Host is a Cloud that does not run yet, read reactively. */
  hostStarting(): boolean
}

/** Answers that will not change by asking again. */
const FINAL_REASONS: ReadonlySet<PanelReason | null> = new Set<PanelReason>(['conversation_not_found', 'conversation_archived'])

/** Whether this web browser can show the view's pictures, once it has answered. */
export type PictureSupport = 'checking' | 'supported' | 'unsupported'

/**
 * What Demi does for a panel tab before the browser has its tab: reads what the
 * Host is doing, starts the Cloud, starts the browser on the Host, or has the
 * browser open the tab.
 */
export type StartingPhase = 'connecting' | 'cloud' | 'browser' | 'page'

/** What the content says in each phase, from the frame the phase begins. */
export function startingLabel(phase: StartingPhase): SentenceText {
  return t(`starting.${phase}`)
}

export interface BrowserTabsOptions {
  /** The page's visibility; the document's own unless a test supplies one. */
  visibility?: Readonly<Ref<DocumentVisibilityState>>
  /** Resolves whether this web browser can decode the view's pictures; WebCodecs' answer unless the gallery or a test supplies one. */
  pictures?: () => Promise<boolean>
}

/**
 * The user's latest request on a browser tab: sent and not answered yet, or
 * answered with the number of the last tab list before it started, which only a
 * list numbered higher outdates. A refused request leaves none.
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

/** A browser tab's last picture, copied off the canvas that showed it, with the generation it was of, which places it. */
export interface KeptPicture {
  image: CanvasImageSource & { width: number; height: number }
  stream: LiveStream
}

/** How the page reports failures: to the user in a toast, or a defect of the page to the console. */
export type PageErrors = PageContext['errors']

/** Whether the page is visible: one listener, for the page's lifetime, that every controller shares. */
const pageVisibility = useDocumentVisibility()

/** How long a panel that resizes waits before the module hears of it: a drag would otherwise resize the capture each step. */
const PANEL_SETTLE_MS = 100

/**
 * One conversation's browser, for one page, made in the effect scope its
 * store gives it. It holds the tab list, each browser tab as a view last
 * reported it with its last picture, the panel the shown tab is sized by, and
 * the view, watched while a `browser` tab's content is shown and the page is
 * visible. It alone tells the view which tab to watch.
 */
export class BrowserTabsController {
  /** The last list the plugin's state gave; null before the first. */
  readonly list: ComputedRef<BrowserTabList | null>
  /** Why the last list could not be read, until one is. */
  readonly listError: ComputedRef<BrowserTabsError | null>
  readonly session: ShallowRef<LiveSession | null> = shallowRef(null)
  /**
   * Whether this web browser can show the pictures, asked once. One that
   * cannot opens no view, so it is no viewer.
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
  /** Each browser tab's last picture, while the browser has the tab. */
  private readonly lastPictures = new Map<string, KeptPicture>()
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
    // The watchers stop with the effect scope its store made it in.
    watch(this.visibility, (state) => this.visibilityChanged(state), { flush: 'sync' })
    // A view the stopped Cloud refused connects at once once the Cloud runs, so
    // the Host knows the panel's size before it opens the tab a shown content
    // waits for.
    watch(() => api.hostStarting(), (starting, was) => {
      if (was && !starting) {
        this.session.value?.reconnect()
      }
    })
    watch(this.listError, (error) => {
      if (error && FINAL_REASONS.has(error.reason)) {
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

  /** The last picture a view of the browser tab `tab` showed, kept for the next view of it. */
  keepPicture(tab: string, picture: KeptPicture): void {
    if (this.known.has(tab)) {
      this.lastPictures.set(tab, picture)
    }
  }

  /** The browser tab's last picture, if a view kept one. */
  keptPicture(tab: string): KeptPicture | null {
    return this.lastPictures.get(tab) ?? null
  }

  /** The downloads the user started in the browser tab `tab`, the newest last. */
  downloadsOf(tab: string | undefined): readonly LiveDownload[] {
    return tab === undefined ? [] : (this.downloads.get(tab) ?? [])
  }

  /**
   * Whether the panel tab bound to `tab`, asking for `url`, shows its page
   * loading.
   *
   * - A request of the user's loads from the moment it is made until a tab list
   *   numbered higher than the one its answer names says otherwise, whichever
   *   reaches the page first. On a far backend the browser may start loading
   *   well after the answer, and a list numbered no higher describes the page
   *   as it was.
   * - Otherwise the page loads while the browser last said it does: in a view,
   *   or, for a tab not shown, in the plugin's tab list.
   * - A shown tab no view reported yet is one whose browser tab is still
   *   opening, and it loads while it asks for an address.
   * - One not shown that the list lacks too was lost with the browser, and
   *   waits, not loading, until it is shown.
   * - A tab shown again keeps what the browser last said, so a page that had
   *   loaded shows no loading.
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
    return url !== BLANK_URL
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
   * What the browser reports of the page the panel tab with `data` shows: as a
   * view last reported it, else as the plugin last listed it. The tab's
   * address and its title both come from this one record, so they never
   * describe two pages. None when:
   *
   * - the tab has no browser tab;
   * - the browser still reports the page a request of the user's leaves:
   *   until a list after the request, while the browser loads from an address
   *   other than the one the tab asks for, as it does until the new page
   *   commits, and while it reports the blank page of a browser tab that has
   *   not started on the address yet.
   *
   * The address and the title then come from the panel tab's own data.
   */
  private reportedPage(data: BrowserTabData): Pick<LiveTab, 'url' | 'title' | 'loading'> | null {
    if (data.tab === undefined || data.closed || data.failure || this.requested(data.tab)) {
      return null
    }
    const reported = this.tab(data.tab) ?? this.list.value?.tabs.find((candidate) => candidate.id === data.tab)
    if (!reported || (reported.loading && reported.url !== data.url)) {
      return null
    }
    return reported.url === BLANK_URL && data.url !== BLANK_URL ? null : reported
  }

  /**
   * The title of the page the panel tab with `data` shows, as the browser
   * reports it, the moment it names it, as a web browser's tab takes its
   * page's title in the background too; none when the page has no title or
   * the browser reports no page of the tab (`reportedPage`).
   */
  title(data: BrowserTabData): string | null {
    return this.reportedPage(data)?.title || null
  }

  /** The address of the page the panel tab with `data` shows: the page's own, else the one the tab asks for (`reportedPage`). */
  address(data: BrowserTabData): string {
    return this.reportedPage(data)?.url ?? data.url
  }

  /**
   * The icon of the page the panel tab with `data` shows, as a web
   * browser's tab shows it: as a view last reported it, else as the plugin
   * last listed it; null without a browser tab, or for a page without one.
   */
  favicon(data: BrowserTabData): string | null {
    if (data.tab === undefined || data.closed || data.failure) {
      return null
    }
    const live = this.tab(data.tab)
    if (live) {
      return live.favicon ?? null
    }
    return this.list.value?.tabs.find((candidate) => candidate.id === data.tab)?.favicon ?? null
  }

  /**
   * Whether the panel tab `panelTab`, with `data`, shows its page loading: the
   * one state its strip's spinner, its Stop and its progress line show.
   *
   * - It loads while its browser tab opens, the first time or again, and
   *   otherwise as its browser tab does.
   * - A shown tab that has no picture yet loads until its first picture, unless
   *   none can come.
   * - A tab that could not open, or whose browser tab the browser lost and no
   *   one opened again yet, does not load, nor does a blank one, which loads
   *   nothing.
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
   * Whether the panel tab with `data` is blank: its address is the blank
   * page's, as after its user stopped it before the browser had it, and no
   * request of the user's is under way on its browser tab. It loads nothing, so
   * it shows a blank page, not what Demi waits for; only the browser's own word
   * that its page loads shows it loading.
   */
  private blank(data: BrowserTabData): boolean {
    return data.url === BLANK_URL && !this.requested(data.tab)
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
   * browser has its tab: while it opens, the first time or again, while it
   * waits to, and while the browser's tab list does not name the browser tab
   * its data names, which the browser may have lost, as after a restart.
   *
   * The phases, in order:
   *
   * 1. The Cloud starting, which the product state says.
   * 2. Connecting, while neither a view nor the plugin's list has said what the
   *    conversation's browser holds.
   * 3. The browser starting, while it has no tabs.
   * 4. The browser opening the tab.
   *
   * Null once the list names the tab, for a tab that could not open and was not
   * retried, and for a blank tab, which waits for nothing the user asked for.
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
    this.history(data.tab, 'reload').catch((error: unknown) => this.errors.report(t('failures.reloadPage'), error))
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
   * Runs a request of the user's on `tab`. The latest one on the tab is the one
   * its loading follows; a refusal ends it at once, and the caller reports it.
   * A tab the browser no longer has was lost with the browser, as when Demi
   * restarted: nothing says so, and the plugin is asked to read the list, which
   * marks the panel tab lost, so it opens again on its address, at once while
   * it is shown, as a browser reloads a discarded tab.
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
      // The browser's own message tells the plugin the tab is gone, and
      // the panel follows.
      if (error instanceof BrowserTabsError && error.reason === 'tab_not_found') {
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
   * when there is none.
   *
   * - One shown before the browser has its tab watches nothing, and the view
   *   tells the Host the panel's size meanwhile.
   * - A view waiting to reconnect connects at once for a tab it did not show,
   *   as a tab that just got its browser tab: the view's end may have been only
   *   that there was no browser yet.
   * - A hidden page opens no view until it is shown, and a web browser that
   *   cannot show the pictures opens none at all.
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
   * shows it in the same flush, before or after this, so the view is
   * unwatched only when none followed.
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
        this.unwatchView()
      }
    }, 0)
  }

  /**
   * Nobody can watch a hidden page, so the view is unwatched while the page
   * is hidden. Shown again, the page watches the shown tab on it.
   */
  private visibilityChanged(state: DocumentVisibilityState): void {
    if (state !== 'visible') {
      this.unwatchView()
      return
    }
    this.watchShown()
  }

  /** The view watches no tab and says nobody watches it, keeping its connection. */
  private unwatchView(): void {
    const view = this.session.value
    view?.watch(null)
    view?.watched(false)
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
    const view = this.view()
    view.watched(true)
    view.watch(this.shownTab.value)
  }

  /**
   * The view's tab list. Each tab is kept as reported, and one the browser no
   * longer has leaves. The panel's tabs follow the plugin, which the browser
   * tells of every change.
   */
  private viewTabs(tabs: readonly LiveTab[], list: number): void {
    this.listed.value = list
    this.pageOpened(tabs)
    for (const id of [...this.known.keys()]) {
      if (!tabs.some((tab) => tab.id === id)) {
        this.known.delete(id)
        this.pictured.delete(id)
        this.lastPictures.delete(id)
      }
    }
    for (const tab of tabs) {
      this.known.set(tab.id, tab)
    }
    // A state replaces the page's copy: the view tells the downloads of the tab
    // it watches again, and the page keeps none of a tab that is gone or that
    // no view watches.
    const watched = this.shownTab.value
    for (const id of [...this.downloads.keys()]) {
      if (id !== watched || !tabs.some((tab) => tab.id === id)) {
        this.downloads.delete(id)
      }
    }
  }

  /**
   * A tab a page opened appears in the strip at once, as in any browser: the
   * plugin adds it when the browser tells it the tabs changed. One that the
   * page the user watches opened right after the user's click or key there is
   * the user's, and is selected, as a browser selects the tab a click opens.
   * One a page opened by itself is only added.
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
    const shown = this.shownTab.value
    const view = this.session.value
    for (const tab of opened) {
      const opener = tab.createdBy.kind === 'page' ? tab.createdBy.opener : null
      if (opener !== null && opener === shown && view?.pressedLately(opener, OPENED_BY_USER_MS)) {
        this.api.select(addedPanelTab(tab.id))
      }
    }
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
      onPicture: (tab) => this.pictured.add(tab),
      onDownloads: (tab, downloads) => this.downloads.set(tab, downloads),
      defect: this.errors.defect,
    })
    this.session.value = session
    session.start()
    return session
  }

  /** A notice the picture still shows through is a failed request: a toast in the Writing page's words for its reason. */
  private notice(reason: PanelReason): void {
    this.errors.report(t('failures.operate'), BrowserTabsError.of(reason))
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
    this.lastPictures.clear()
    this.downloads.clear()
    this.closeView()
  }
}

/** A failure that is no answer of the backend's, such as a network failure or a timeout, as the page says it. */
export function asTabsError(error: unknown): BrowserTabsError {
  return error instanceof BrowserTabsError ? error : new BrowserTabsError({ answer: 'none' })
}
