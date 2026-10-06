import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { browserPage } from '@demicodes/plugin-browser'
import { PanelTabs, closePanelTabs, updatePanelTab } from '@demicodes/web-ui/agent/panel-changes'
import {
  openIntent,
  pendingShows,
  shownSelection,
  type AppliedShows,
  type PanelState,
  type PinnedTabs,
} from '@demicodes/web-ui/agent/panel-tabs'
import { selectTab } from '@demicodes/web-ui/agent/tab-close'
import type { CallEditSelection } from '@demicodes/web-ui/files/changes'
import type { IntentRequest } from '@demicodes/web-ui/plugins/intents'
import {
  bindPages,
  intentKind,
  type AnyPluginPage,
  type ConversationFileService,
} from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../generated/pages'
import { productWould } from '../product-would'
import { readGalleryEdit } from './blobs'
import { galleryBrowser, type GalleryBrowser } from './live-browser'
import { GalleryPanel, galleryBrowserPlugin } from './panel'
import type { createGalleryWorkspace } from './workspace'
import { browserPlugin, galleryPageHost } from './plugins'

/** The gallery workspace as the work panel's kinds read a conversation's files. */
export function galleryFiles(workspace: ReturnType<typeof createGalleryWorkspace>): ConversationFileService {
  return {
    workspace: { source: workspace.source, root: workspace.root },
    root: workspace.root,
    changes: workspace.changes,
    edit: readGalleryEdit,
    // The gallery's working tree changes only when a specimen changes it.
    showChanges: () => {},
  }
}

/** The conversation every gallery panel shows. */
const CONVERSATION = 'gallery'

/**
 * One work panel the way the product's work store holds it
 * (`web-application.md` § Work panel): its selection history, the pinned tabs' data,
 * and the tabs of a panel kept as the backend keeps them, with the
 * specimen's changes shown at once over them; the plugin pages' kinds are
 * bound to it over the specimen's files and its own conversation browser,
 * whose plugin opens and closes browser tabs for the panel's tabs as the
 * backend's does. Intents open in it as they open in the product.
 */
export function useGalleryWork(
  selection: string | null,
  { files, browser = galleryBrowser(), pictures, pages }: {
    files: ConversationFileService
    browser?: GalleryBrowser
    /** Whether the web browser decodes the pictures; by default it asks the web browser, as the product does. */
    pictures?: () => Promise<boolean>
    /** The plugin pages whose kinds the panel shows; every one the product shows by default. */
    pages?: readonly AnyPluginPage[]
  },
) {
  // The gallery's pages, with the browser's own made for a specimen that says how the pictures decode.
  const shown = pages ?? PLUGIN_PAGES.map((page) => (page.plugin === 'browser' && pictures ? browserPage({ pictures }) : page))
  const enabled = () => true
  const backend = new GalleryPanel()
  const plugin = galleryBrowserPlugin(browser, backend)
  // The browser's tabs stand in the panel in its order, as the product's would: the user's with the tab the
  // user made for each, the agent's with the id the plugin gives them. A panel without the browser's page has none.
  if (shown.some((page) => page.plugin === 'browser')) {
    for (const tab of browser.listed.value.tabs) {
      const id = tab.createdBy.kind === 'user' ? `user-${tab.id}` : `browser-${tab.id}`
      backend.apply({ type: 'create', tab: { id, kind: 'browser', data: { url: tab.url, tab: tab.id, title: tab.title } } })
    }
  }
  const tabs = new PanelTabs(backend, (error) => {
    productWould(`Report the Refused Panel Change: ${error instanceof Error ? error.message : String(error)}`)
  })
  backend.told = ({ change, tab }) => {
    if (tab.kind !== 'browser') {
      return
    }
    void (change === 'created' ? plugin.bind(tab.id) : plugin.removed(tab))
  }
  backend.changed = (revision) => tabs.noticed(revision)
  tabs.start()

  const history = ref<readonly string[]>(selection === null ? [] : [selection])
  const pinned = ref<PinnedTabs>({})
  const panel = computed<PanelState>(() => ({ history: history.value, tabs: tabs.tabs.value }))

  function select(id: string) {
    history.value = selectTab(history.value, id)
  }
  /** Each tab's showings the panel applied, as the product keeps them beside the history. */
  let applied: AppliedShows = {}
  /** Whether the specimen's panel is open, which a tab its kind asks to show opens. */
  const open = ref(true)
  // A tab its kind asks to show opens the panel and is selected once, as the product's work store does it.
  watch(() => tabs.tabs.value, (current) => {
    const pending = pendingShows(current, shown, enabled, applied)
    if (!pending) {
      return
    }
    applied = pending.applied
    for (const id of pending.shown) {
      select(id)
    }
    open.value = true
  })
  /** A new tab after the others, selected unless `options` says not; returns its id. */
  function add(kind: string, data: unknown, options = { select: true }): string {
    const id = crypto.randomUUID()
    tabs.change({ type: 'create', tab: { id, kind, data } })
    if (options.select) {
      select(id)
    }
    return id
  }
  function update(id: string, data: unknown) {
    updatePanelTab(tabs, id, data)
  }
  function updatePinned(kind: string, data: unknown) {
    pinned.value = { ...pinned.value, [kind]: data }
  }
  /** Closed tabs go at once; the panel shows what was selected before a closed one. */
  function closeTabs(ids: string[]) {
    const selection = {
      get history() {
        return history.value
      },
      set history(next: readonly string[]) {
        history.value = next
      },
    }
    closePanelTabs(selection, tabs, ids)
  }
  function openIn(request: IntentRequest) {
    const opened = openIntent(pinned.value, shown, enabled, request)
    if (!opened) {
      return
    }
    pinned.value = opened.pinned
    if (opened.created) {
      tabs.change({ type: 'create', tab: opened.created })
    }
    select(opened.selection)
  }

  const host = galleryPageHost({ browser: browserPlugin(browser, plugin) }, {
    files,
    intents: {
      open: (_conversation, request) => openIn(request),
      canOpen: (intent) => intentKind(shown, enabled, intent) !== null,
    },
    panel: {
      tabs: (_conversation, kind) => tabs.tabs.value.filter((tab) => tab.kind === kind).map((tab) => tab.data),
      add: (_conversation, kind, data, options = { select: false }) => void add(kind, data, options),
    },
  })
  const bound = bindPages(shown, host, CONVERSATION)
  onBeforeUnmount(() => bound.dispose())
  const kinds = bound.kinds
  /** What the panel shows, as its strip marks it. */
  const selected = computed(() => shownSelection(panel.value, kinds))

  /** A tool call's file pill, through the `edit` intent. */
  function selectEdit(edit: CallEditSelection) {
    openIn({ intent: 'edit', payload: edit })
  }
  /** The panel as it started: every tab closed, the first selection again. */
  function reset() {
    closeTabs(tabs.tabs.value.map((tab) => tab.id))
    history.value = selection === null ? [] : [selection]
    pinned.value = {}
  }
  /**
   * The agent shows `tab` with `demi browser show`, and its job ends: the
   * plugin reads the tab list and carries the count into the panel tab,
   * which the panel then selects.
   */
  async function agentShows(tab: string) {
    browser.show(tab)
    await plugin.sync()
  }
  /** The agent opens a page, shown with `--show` or not, and its job ends: the plugin adds the tab. */
  async function agentOpens(url: string, show: boolean) {
    browser.agentOpens(url, { show })
    await plugin.sync()
  }
  return {
    panel,
    pinned,
    open,
    kinds,
    selected,
    browser,
    agentShows,
    agentOpens,
    host,
    select,
    add,
    update,
    updatePinned,
    closeTabs,
    openIn,
    selectEdit,
    reset,
  }
}

export type GalleryWork = ReturnType<typeof useGalleryWork>
