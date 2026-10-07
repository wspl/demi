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
import type { PanelSession, PreviewPlace } from '@demicodes/plugin-sdk'
import type { PreviewOpened } from '../generated/plugin'
import { previewUnsupported } from './client'
import type { PreviewConnection } from './connection'
import type { Navigation, RelayBinding, RelayTab, TabEvent, TabPage } from './relay'

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
  readonly view: PreviewTabView = reactive({ src: null, page: null, icon: null, loading: false, failure: null, starting: false })
  opener: { binding: RelayBinding; popup: number } | null = null
  /** The navigation the tab loads, which Retry opens again. */
  private current: Navigation | null = null
  /** Counts navigations, so an older one's late answer changes nothing. */
  private generation = 0
  /** The frame shows the boot page, whose own load is no page loaded. */
  private booting = false
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

  start(): void {
    this.unregister = this.tabs.driver.register(this)
  }

  stop(): void {
    this.unregister()
    // A page that opened this tab hears that it closed.
    this.opener?.binding.port.postMessage({ type: 'tab-closed', id: this.opener.popup })
  }

  /** Loads `navigation`: the user's own when it has no initiator. */
  async load(navigation: Navigation): Promise<void> {
    const generation = ++this.generation
    const place = this.place()
    this.current = navigation
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

  /** The address the user entered: the tab keeps it at once and loads it. */
  submit(url: string): void {
    const { title: _left, ...data } = this.host.data()
    this.host.update({ ...data, url })
    void this.load({ url, initiator: null })
  }

  /** Back, Forward or Reload in the page; Reload of a page that failed opens its address again. */
  history(action: 'back' | 'forward' | 'reload'): void {
    if (this.tabs.driver.command(this, action)) {
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
    this.view.loading = false
    this.view.starting = false
    if (this.view.page && this.tabs.driver.command(this, 'stop')) {
      return
    }
    this.view.src = null
    this.view.page = null
    this.view.failure = null
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
      case 'leaving':
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
