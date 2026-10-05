/**
 * The `browser` page's panel session (`live-view.md` § A browser tab in the
 * panel): what a tab keeps, the conversation browser's tab list for the
 * tabs' titles, which the plugin's conversation state brings, the requests
 * that move a bound tab, and the one view a page keeps while a `browser` tab
 * is shown and the page is visible. Which browser tab a panel tab shows is
 * the plugin's work on the backend; the session never opens, closes or adds
 * a tab.
 */
import { clientPlatform } from '@demicodes/utils'
import { useDocumentVisibility } from '@vueuse/core'
import { computed, shallowRef, watch, type ComputedRef, type Ref, type ShallowRef } from 'vue'
import { z } from 'zod'
import type { BrowserTab, NeededBrowser } from '../generated/plugin'
import { viewerClipboard } from './clipboard'
import { picturesSupported } from './pictures'
import type { HostArtifact, HostInstall, OpenUserStream, SentenceText } from '@demicodes/plugin-sdk'
import { LiveSession } from './session'

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
  navigate(tab: string, url: string): Promise<void>
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<void>
  stream: OpenUserStream
  /** What the Host installs before the browser can start, read reactively. */
  installs(): readonly HostInstall[]
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

/** Reports a defect of the page itself, as the page context's `errors.defect` does. */
export type ReportDefect = (message: string, error: unknown) => void

/** Whether the page is visible: one listener, for the page's lifetime, that every controller shares. */
const pageVisibility = useDocumentVisibility()

/**
 * One conversation's browser, for one page, made by the page's panel
 * session in its effect scope: the tab list it last learned, for the tabs'
 * titles, and the view, open only while a `browser` tab's content is shown
 * and the page is visible (`live-view.md` § Ending a view).
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
  /** The tabs as the open view last reported them, the newest there are; none while no view is open. */
  private readonly viewed: ShallowRef<readonly BrowserTab[] | null> = shallowRef(null)
  /** The browser tab whose content is shown, which the view watches while the page is visible. */
  private shown: string | null = null
  /** The shown tab the view found gone, which the plugin was asked to look for once. */
  private missed: string | null = null
  private closing: ReturnType<typeof setTimeout> | null = null
  private readonly visibility: Readonly<Ref<DocumentVisibilityState>>
  private disposed = false

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

  /**
   * The tab `tab` as the browser names it, if it does: as the open view
   * reported it, which follows every page, else as the plugin's state last
   * listed it. It names a tab's title only, so the newer of the two is
   * enough.
   */
  listed(tab: string | undefined): BrowserTab | null {
    const tabs = this.viewed.value ?? this.list.value?.tabs ?? []
    return tabs.find((listed) => listed.id === tab) ?? null
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

  /** The shown tab on the page's one view, while someone can see its pictures. */
  private watchShown(): void {
    if (this.shown === null || this.visibility.value !== 'visible' || this.pictures.value !== 'supported') {
      return
    }
    this.view().watch(this.shown)
  }

  /**
   * The view's tab list. A list without the shown tab asks the plugin, once
   * for that tab, to read the browser's tabs: the plugin marks the panel tab
   * closed if the browser lost it.
   */
  private viewTabs(tabs: readonly BrowserTab[]): void {
    this.viewed.value = tabs
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
      platform: clientPlatform(navigator),
      onClipboard: (text) => viewerClipboard.receive(text),
      onTabs: (tabs) => this.viewTabs(tabs),
      defect: this.defect,
    })
    this.session.value = session
    session.start()
    return session
  }

  private closeView(): void {
    this.session.value?.close()
    this.session.value = null
    this.viewed.value = null
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
