/**
 * The tabs of the user's browser of one conversation (`preview.md` § What
 * the user sees, § Opening and navigating): each shows its pages in a frame
 * of this page, which the relay serves from the conversation's Host. A tab
 * is a panel tab of the `preview` kind; what it shows lives here while the
 * conversation's panel is open, and its data keeps the address and title,
 * so a reload opens it again where it was.
 */
import { computed, reactive, shallowReactive, type ComputedRef } from 'vue'
import { z } from 'zod'
import type { HeadlineText, PanelSession, PreviewPlace, SentenceText } from '@demicodes/plugin-sdk'
import { PREVIEW_MAX_STORAGE_BYTES, type PageStorage, type PreviewOpened } from '../generated/plugin'
import type { BrowserTabData } from '../live/tabs'
import { previewUnsupported } from './client'
import type { PreviewConnection, TakenState } from './connection'
import type { Navigation, NavigationType, RelayBinding, RelayTab, TabEvent, TabPage } from './relay'

/** A tab of the user's browser as its panel tab keeps it. */
export const previewTabDataSchema = z.object({
  /** The real address the tab shows; empty for a new tab. */
  url: z.string(),
  /** The page's title as the tab last showed it. */
  title: z.string().optional(),
  /** The tab whose page opened this one, which places it beside that tab. */
  openedBy: z.string().optional(),
  /** Which window of its opener's page this tab is, while that page lives. */
  window: z.string().optional(),
  /**
   * The tab of the agent's browser whose page this tab opens, with its page
   * state (`preview.md` § Page state); dropped once the tab took it, so a
   * reload opens the address alone.
   */
  from: z.string().optional(),
  /** The tab shows its pages as a phone does, Mobile in its size menu (`preview.md` § Mobile); Web without it. */
  mobile: z.literal(true).optional(),
})
export type PreviewTabData = z.infer<typeof previewTabDataSchema>

/** What reaches the Host and the relay for a conversation's tabs. */
export interface PreviewApi {
  /** Where the conversation's previews live; null while they cannot. */
  readonly place: ComputedRef<PreviewPlace | null>
  /** The conversation's `preview` stream. */
  readonly connection: PreviewConnection
  /** Whether the conversation's Host is a Cloud that is starting. */
  hostStarting(): boolean
  /** The top-level label of an address, from the Host, which wakes a stopped Cloud. */
  open(url: string, place: PreviewPlace): Promise<PreviewOpened>
  /** Adds a tab of the user's browser to the panel. */
  add(data: PreviewTabData, select: boolean): void
  /** Adds a tab of the agent's browser to the panel. */
  addAgentTab(data: BrowserTabData, select: boolean): void
  /** Tells the user a fact about what they asked for, as a neutral toast. */
  notify(title: HeadlineText, message: SentenceText): void
}

/**
 * How a tab's frame reaches its pages: in the product, the relay over the
 * conversation's Host; in the gallery, its fixtures.
 */
export interface PreviewDriver {
  /** Starts relaying for `tab` until the returned function is called. */
  register(tab: PreviewTab): () => void
  /** The frame's address that opens `navigation` in `tab`, whose top-level label `opened` names. */
  boot(tab: PreviewTab, place: PreviewPlace, opened: PreviewOpened, navigation: Navigation): Promise<string>
  /** Asks the tab's page for Back, Forward, Reload or Stop; false while it shows no page of the preview. */
  command(tab: PreviewTab, command: 'back' | 'forward' | 'reload' | 'stop'): boolean
  /** The page's icon as an address this page can show, or null. */
  icon(tab: PreviewTab, place: PreviewPlace, page: TabPage): Promise<string | null>
  /** Whether a frame first loads the boot page, whose own load is no page loaded. */
  readonly boots: boolean
  /** The tab's frame finished loading a document. */
  loaded?(tab: PreviewTab): void
  /**
   * The page state of the agent's tab `from`: its cookies moved into the
   * jar, and its address, title and top-level origin's storage. Rejects with
   * why, in a sentence the user reads.
   */
  takeState(tab: PreviewTab, place: PreviewPlace, from: string): Promise<TakenState>
  /** Writes `storage` into the top-level origin `opened` names, before the page loads; answers what another page kept. */
  writeState(tab: PreviewTab, opened: PreviewOpened, storage: PageStorage): Promise<string[]>
  /** The storage of the tab's top-level origin; null while the tab shows no page of the preview. */
  readState(tab: PreviewTab): Promise<PageStorage | null>
  /** The real origins of the tab's documents, whose sites' cookies a page state moves with it. */
  origins(tab: PreviewTab): string[]
  /** Keeps a page state of the tab under `token`, for the tab of the agent's browser that opens with it. */
  keepState(tab: PreviewTab, place: PreviewPlace, token: string, origins: string[], storage: PageStorage | null): Promise<void>
}

/** What a toast says when a page's storage moved only in part (`preview.md` § Page state). */
const PART_MOVED: HeadlineText = 'Some of the Page’s Storage Didn’t Move'
/** What a toast says when a page moved without its state. */
const NOT_MOVED: HeadlineText = 'The Page Opened Without Its State'
/** What a toast says when a page's storage was too large to move. */
const COOKIES_ONLY: HeadlineText = 'Only the Page’s Cookies Moved'
const TOO_LARGE: SentenceText = 'Its storage is larger than the 16 MB a page state moves.'

/** The sentence that names what of `origin`'s storage was left out: stores with values that cannot be copied, and databases another page held. */
function leftOut(origin: string, skipped: readonly string[], kept: readonly string[]): SentenceText | null {
  const parts = [
    ...skipped.map((store) => `${store} has a value that can’t be copied`),
    ...kept.map((database) => `${database} is open in another tab`),
  ]
  return parts.length === 0 ? null : `On ${URL.parse(origin)?.host ?? origin}, ${parts.join('; ')}.`
}

/** The sentence a failure of a page state says: its reason, as a sentence. */
function failedSentence(error: unknown): SentenceText {
  const text = error instanceof Error ? error.message : String(error)
  const sentence = text.charAt(0).toUpperCase() + text.slice(1)
  return /[.!?]$/.test(sentence) ? sentence : `${sentence}.`
}

/** What a tab shows: its frame's address, the page it reports, and whether it loads or failed. */
export interface PreviewTabView {
  /** The frame's address; null while the tab shows no page, as a new tab. */
  src: string | null
  /** The page as its top document last told it. */
  page: TabPage | null
  /** The page's icon, as the strip shows it; null for none. */
  icon: string | null
  loading: boolean
  /** Why the tab's page could not open; null while it did not fail. */
  failure: string | null
  /** The tab waits for the Host: its Cloud starts. */
  starting: boolean
  /** The tab's history has a page before the one it shows, of any origin. */
  canGoBack: boolean
  /** The tab's history has a page after the one it shows. */
  canGoForward: boolean
}

/**
 * The entries of a tab's frame's history, of every origin its pages had, as
 * its top documents report each move: a browser's Back and Forward count
 * them, and the Navigation API in a page sees only its own origin's.
 */
export class TabHistory {
  private entries: string[] = []
  private index = -1

  /** The history moved to the entry `key`, as `navigationType` says. */
  moved(key: string, navigationType: NavigationType): void {
    const known = navigationType === 'traverse' ? this.entries.indexOf(key) : -1
    if (known >= 0) {
      this.index = known
    } else if (this.index >= 0 && (navigationType === 'replace' || navigationType === 'reload')) {
      this.entries[this.index] = key
    } else {
      // A new page; an entry the list never met, as one before the tab's content started, counts as one.
      this.entries = [...this.entries.slice(0, this.index + 1), key]
      this.index = this.entries.length - 1
    }
  }

  get canGoBack(): boolean {
    return this.index > 0
  }

  get canGoForward(): boolean {
    return this.index < this.entries.length - 1
  }
}

/** What a tab's content gives its tab: its frame, and how to change and close the panel tab. */
export interface PreviewTabHost {
  frame(): HTMLIFrameElement | null
  data(): PreviewTabData
  update(data: PreviewTabData): void
  close(): void
}

/**
 * Why a page could not open, in the words the tab shows: the engine's
 * reasons are its own (`preview.md` § The preview engine), and the rest is
 * the Host's network, or the stream to it.
 */
export function failureSentence(reason: string): string {
  if (reason === 'refused: local-network') {
    return 'A page can’t open an address in a more private network than its own.'
  }
  if (reason.startsWith('refused:')) {
    return 'The page’s site refused the request.'
  }
  if (reason === 'host_stopped') {
    return 'The Cloud is stopped.'
  }
  if (reason === 'device_offline' || reason === 'host_unreachable') {
    return 'The device can’t be reached.'
  }
  return 'The Host couldn’t reach this address.'
}

/** A window a page opened, which its new tab takes when its content starts. */
interface OpenedWindow {
  opener: RelayBinding
  popup: number
  navigation: Navigation | null
}

/** One tab of the user's browser while its content lives. */
export class PreviewTab implements RelayTab {
  readonly view: PreviewTabView = reactive({
    src: null,
    page: null,
    icon: null,
    loading: false,
    failure: null,
    starting: false,
    canGoBack: false,
    canGoForward: false,
  })
  private entries = new TabHistory()
  opener: { binding: RelayBinding; popup: number } | null = null
  /** The navigation the tab loads, which Retry opens again. */
  private current: Navigation | null = null
  /** Counts navigations, so an older one's late answer changes nothing. */
  private generation = 0
  /** The frame shows the boot page, whose own load is no page loaded. */
  private booting = false
  /**
   * The page took a Back, Forward or Reload and has not said yet whether its
   * document goes, which the next load ends, or stays, which its history's
   * move ends.
   */
  private commanded = false
  /** The page icon's address the tab shows, or fetches. */
  private iconOf = ''
  private unregister: () => void = () => {}

  constructor(
    readonly id: string,
    private readonly tabs: PreviewTabs,
    private readonly host: PreviewTabHost,
  ) {}

  get connection(): PreviewConnection {
    return this.tabs.api.connection
  }

  place(): PreviewPlace | null {
    return this.tabs.api.place.value
  }

  frame(): HTMLIFrameElement | null {
    return this.host.frame()
  }

  frameWindow(): Window | null {
    return this.host.frame()?.contentWindow ?? null
  }

  mobile(): boolean {
    return this.host.data().mobile === true
  }

  /**
   * Web or Mobile, as the size menu chooses: the tab keeps it, and its page
   * loads again, since a phone's user agent and hints reach only what loads
   * after them (`live-view.md` § Modes).
   */
  setMobile(mobile: boolean): void {
    if (mobile === this.mobile()) {
      return
    }
    const { mobile: _was, ...data } = this.host.data()
    this.host.update(mobile ? { ...data, mobile: true } : data)
    if (this.view.page) {
      this.history('reload')
    }
  }

  start(): void {
    this.unregister = this.tabs.driver.register(this)
  }

  stop(): void {
    this.unregister()
    // A page that opened this tab hears that it closed.
    this.opener?.binding.port.postMessage({ type: 'tab-closed', id: this.opener.popup })
  }

  /**
   * Loads `navigation`: the user's own when it has no initiator. With
   * `from`, a tab of the agent's browser, its page state is in place before
   * the page loads (`preview.md` § Page state).
   */
  async load(navigation: Navigation, from?: string): Promise<void> {
    const generation = ++this.generation
    const place = this.place()
    this.current = navigation
    this.commanded = false
    this.view.failure = null
    // Where previews cannot open, the tab says why in place of its page.
    if (!place || this.tabs.unavailable.value) {
      this.view.loading = false
      return
    }
    this.view.loading = true
    this.view.starting = this.tabs.api.hostStarting()
    try {
      const opened = await this.tabs.api.open(navigation.url, place)
      if (generation !== this.generation) {
        return
      }
      if (from !== undefined) {
        await this.receiveState(place, opened, from)
        if (generation !== this.generation) {
          return
        }
      }
      const src = await this.tabs.driver.boot(this, place, opened, navigation)
      if (generation !== this.generation) {
        return
      }
      this.booting = this.tabs.driver.boots
      this.view.src = src
    } catch (error) {
      if (generation === this.generation) {
        this.view.loading = false
        this.view.failure = error instanceof Error ? error.message : String(error)
      }
    } finally {
      if (generation === this.generation) {
        this.view.starting = false
      }
    }
  }

  /**
   * Takes the page state of the agent's tab `from` into the top-level origin
   * `opened` names. It moves once: the tab's data drops `from` at once. What
   * did not move is a toast; the page loads either way.
   */
  private async receiveState(place: PreviewPlace, opened: PreviewOpened, from: string): Promise<void> {
    const { from: _taken, ...data } = this.host.data()
    this.host.update(data)
    try {
      const taken = await this.tabs.driver.takeState(this, place, from)
      // The page shows as the agent's tab showed it, Web or Mobile, from its first request.
      const { mobile: _was, ...rest } = this.host.data()
      this.host.update(taken.mobile ? { ...rest, mobile: true } : rest)
      if (taken.tooLarge) {
        this.tabs.api.notify(COOKIES_ONLY, TOO_LARGE)
      }
      if (taken.storage) {
        const kept = await this.tabs.driver.writeState(this, opened, taken.storage)
        const sentence = leftOut(taken.storage.origin, taken.storage.skipped, kept)
        if (sentence) {
          this.tabs.api.notify(PART_MOVED, sentence)
        }
      }
    } catch (error) {
      this.tabs.api.notify(NOT_MOVED, failedSentence(error))
    }
  }

  /** The address the user entered: the tab keeps it at once and loads it. */
  submit(url: string): void {
    const { title: _left, ...data } = this.host.data()
    this.host.update({ ...data, url })
    void this.load({ url, initiator: null })
  }

  /** Back, Forward or Reload in the page; Reload of a page that failed opens its address again. */
  history(action: 'back' | 'forward' | 'reload'): void {
    // The page's own history would move the Demi page's, past the tab's first page.
    if ((action === 'back' && !this.view.canGoBack) || (action === 'forward' && !this.view.canGoForward)) {
      return
    }
    if (this.tabs.driver.command(this, action)) {
      // The tab loads from the click on, as a browser's does: over a far relay the page's next
      // document may take a while to answer (`live-view.md` § A browser tab in the panel).
      this.view.loading = true
      this.commanded = true
      return
    }
    if (action === 'reload') {
      this.retry()
    }
  }

  /** Opens the tab's address again, after a failure. */
  retry(): void {
    const url = this.current?.url ?? this.host.data().url
    if (url) {
      void this.load(this.current ?? { url, initiator: null })
    }
  }

  /**
   * Stop: the page stops loading; a tab that shows no page yet gives up its
   * address and opens blank, as stopping a page before it shows anything
   * leaves a browser's tab blank.
   */
  halt(): void {
    this.generation++
    this.commanded = false
    this.view.loading = false
    this.view.starting = false
    if (this.view.page && this.tabs.driver.command(this, 'stop')) {
      return
    }
    this.view.src = null
    this.view.page = null
    this.view.failure = null
    // The blank page starts the tab's history again: what came before is not the tab's to go back to.
    this.entries = new TabHistory()
    this.view.canGoBack = false
    this.view.canGoForward = false
    const { title: _left, ...data } = this.host.data()
    this.host.update({ ...data, url: '' })
  }

  /**
   * The frame finished loading: the page it shows, or the browser's error
   * page. The boot page's own load comes first, and the page it opens
   * loads after it.
   */
  loaded(): void {
    if (this.view.src === null) {
      return
    }
    this.tabs.driver.loaded?.(this)
    if (this.booting) {
      this.booting = false
      return
    }
    this.view.loading = false
  }

  report(event: TabEvent): void {
    switch (event.type) {
      case 'page': {
        this.view.page = event.page
        this.view.failure = null
        const data = this.host.data()
        if (data.url !== event.page.url || (data.title ?? '') !== event.page.title) {
          const { title: _left, ...rest } = data
          this.host.update(event.page.title ? { ...rest, url: event.page.url, title: event.page.title } : { ...rest, url: event.page.url })
        }
        void this.showIcon(event.page)
        return
      }
      case 'entry':
        this.entries.moved(event.key, event.navigationType)
        this.view.canGoBack = this.entries.canGoBack
        this.view.canGoForward = this.entries.canGoForward
        // A Back or Forward within the document: its move is all there is to load.
        if (this.commanded) {
          this.commanded = false
          this.view.loading = false
        }
        return
      case 'leaving':
        this.commanded = false
        this.view.loading = true
        return
      case 'failed':
        this.view.loading = false
        this.view.failure = failureSentence(event.reason)
        return
    }
  }

  private async showIcon(page: TabPage): Promise<void> {
    const place = this.place()
    if (!place || !page.icon) {
      this.iconOf = ''
      this.view.icon = null
      return
    }
    if (page.icon === this.iconOf) {
      return
    }
    this.iconOf = page.icon
    // A page without an icon of its own shows the mark.
    const icon = await this.tabs.driver.icon(this, place, page).catch(() => null)
    if (this.iconOf === page.icon) {
      this.view.icon = icon
    }
  }

  openWindow(opener: RelayBinding, popup: number, navigation: Navigation | null): void {
    this.tabs.openWindow(this.id, { opener, popup, navigation })
  }

  navigate(navigation: Navigation): void {
    const { title: _left, ...data } = this.host.data()
    this.host.update({ ...data, url: navigation.url })
    void this.load(navigation)
  }

  close(): void {
    this.host.close()
  }

  /** Starts the tab where its data says: the window its opener asked for, or its address. */
  begin(): void {
    const data = this.host.data()
    if (data.from !== undefined && data.url) {
      void this.load({ url: data.url, initiator: null }, data.from)
      return
    }
    const opened = data.window ? this.tabs.takeWindow(data.window) : undefined
    if (opened) {
      this.opener = { binding: opened.opener, popup: opened.popup }
      if (opened.navigation) {
        void this.load(opened.navigation)
      }
      return
    }
    if (data.url) {
      void this.load({ url: data.url, initiator: null })
    }
  }
}

/** The tabs of the user's browser of one conversation, as its panel session holds them. */
export class PreviewTabs implements PanelSession {
  /** Read reactively: the strip shows a tab's icon and loading once its content started. */
  private readonly tabs = shallowReactive(new Map<string, PreviewTab>())
  private readonly windows = new Map<string, OpenedWindow>()
  /** Why a tab of the user's browser cannot open here, or null when it can. */
  readonly unavailable: ComputedRef<string | null>

  constructor(
    readonly api: PreviewApi,
    readonly driver: PreviewDriver,
    unsupported: () => string | null = previewUnsupported,
  ) {
    this.unavailable = computed(
      () => unsupported() ?? (api.place.value ? null : 'Previews aren’t available on this server yet.'),
    )
  }

  /** The tab `id`, while its content lives. */
  tab(id: string): PreviewTab | undefined {
    return this.tabs.get(id)
  }

  /** A tab's content started: its tab relays and opens where its data says. */
  attach(id: string, host: PreviewTabHost): PreviewTab {
    const tab = new PreviewTab(id, this, host)
    this.tabs.set(id, tab)
    tab.start()
    tab.begin()
    return tab
  }

  /** A tab's content ended. */
  detach(id: string): void {
    this.tabs.get(id)?.stop()
    this.tabs.delete(id)
  }

  /** A page opened a window: a new tab beside its opener, selected, as a browser opens one. */
  openWindow(opener: string, window: OpenedWindow): void {
    const id = crypto.randomUUID()
    this.windows.set(id, window)
    this.api.add({ url: window.navigation?.url ?? '', openedBy: opener, window: id }, true)
  }

  /**
   * Open in Your Browser on the agent's tab `agentTab`, a panel tab of the
   * agent's browser: a tab of the user's browser beside it, selected, which
   * takes the page with its state (`preview.md` § What the user sees).
   */
  fromAgent(agentTab: string, data: BrowserTabData): void {
    if (data.tab === undefined) {
      return
    }
    this.api.add({ url: data.url, ...(data.title ? { title: data.title } : {}), openedBy: agentTab, from: data.tab }, true)
  }

  /**
   * Open in Agent's Browser on the tab `id`: a tab of the agent's browser
   * beside it, selected, at once, as a browser's new tab is, which shows the
   * page opening while the page's state is read here and kept on the Host
   * under the tab's token; the agent's browser writes it before it loads the
   * page (`preview.md` § Page state). A state that cannot be read moves
   * nothing, and the tab opens the address alone; what did not move is a
   * toast.
   */
  async toAgent(id: string): Promise<void> {
    const tab = this.tabs.get(id)
    const place = this.api.place.value
    const url = tab?.view.page?.url
    if (!tab || !place || !url) {
      return
    }
    const handover = crypto.randomUUID()
    this.api.addAgentTab({ url, openedBy: id, handover, ...(tab.mobile() ? { mobile: true } : {}) }, true)
    let origins = this.driver.origins(tab)
    let storage: PageStorage | null = null
    try {
      storage = await this.driver.readState(tab)
      if (storage && new TextEncoder().encode(JSON.stringify(storage)).length > PREVIEW_MAX_STORAGE_BYTES) {
        storage = null
        this.api.notify(COOKIES_ONLY, TOO_LARGE)
      }
      const skipped = storage && leftOut(storage.origin, storage.skipped, [])
      if (skipped) {
        this.api.notify(PART_MOVED, skipped)
      }
    } catch (error) {
      // The tab opens its address alone: no cookies without the storage that goes with them.
      origins = []
      this.api.notify(NOT_MOVED, failedSentence(error))
    }
    try {
      await this.driver.keepState(tab, place, handover, origins, storage)
    } catch (error) {
      // The agent's browser waits a moment for the state, then opens the address alone.
      this.api.notify(NOT_MOVED, failedSentence(error))
    }
  }

  takeWindow(id: string): OpenedWindow | undefined {
    const window = this.windows.get(id)
    this.windows.delete(id)
    return window
  }

  dispose(): void {
    for (const id of [...this.tabs.keys()]) {
      this.detach(id)
    }
    this.api.connection.close()
  }
}
