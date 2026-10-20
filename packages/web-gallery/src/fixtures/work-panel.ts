import { computed, nextTick, reactive, ref, watch } from 'vue'
import { browserPage } from '@demicodes/plugin-browser'
import { mobilePage } from '@demicodes/plugin-mobile'
import { addedPanelTab } from '@demicodes/plugin-browser/live/tabs'
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
import type { RequestEditSelection } from '@demicodes/web-ui/files/request-changes'
import type { IntentRequest } from '@demicodes/web-ui/plugins/intents'
import type { TabUpdate } from '@demicodes/web-ui/plugins/page'
import { disposePluginStores } from '@demicodes/web-ui/plugins/stores'
import {
  intentKind,
  useBoundKinds,
  type AnyPluginPage,
  type ConversationFileService,
} from '@demicodes/web-ui/plugins/page'
import type { ChangeSetSource } from '@demicodes/web-ui/files/changes'
import { PLUGIN_PAGES } from '../generated/pages'
import { productWould } from '../product-would'
import { readGalleryEdit } from './blobs'
import { galleryRequest } from './transcripts'
import { galleryBrowser, type GalleryBrowser } from './live-browser'
import { galleryMobile } from './live-mobile'
import { GalleryPanel, galleryBrowserPlugin } from './panel'
import { createSlotChangeSet, type createGalleryWorkspace } from './workspace'
import { sharedHistory } from './history'
import { galleryHistory } from './browser-history'
import { browserPlugin, galleryPageHost } from './plugins'

/** The gallery workspace as the work panel's kinds read a conversation's files. */
export function galleryFiles(workspace: ReturnType<typeof createGalleryWorkspace>): ConversationFileService {
  // The other checkout's working tree, made once a view shows it.
  let slot: ChangeSetSource | null = null
  return {
    workspace: { source: workspace.source, root: workspace.root },
    root: workspace.root,
    changes: (directory) => directory === undefined || directory === workspace.root ? workspace.changes : (slot ??= createSlotChangeSet(200)),
    // One repository stands behind every working tree of the gallery.
    history: () => sharedHistory.source,
    edit: readGalleryEdit,
    request: galleryRequest,
    // The gallery's working tree changes only when a specimen changes it.
    showChanges: () => {},
  }
}

/** The conversation every gallery panel shows. */
const CONVERSATION = 'gallery'

/**
 * One work panel the way the product's work store holds it:
 *
 * - Its selection history, the pinned tabs' data, and the tabs of a panel kept
 *   as the backend keeps them, with the specimen's changes shown at once over
 *   them.
 * - The plugin pages' kinds, bound to it over the specimen's files and its own
 *   conversation browser, whose plugin opens and closes browser tabs for the
 *   panel's tabs as the backend's does.
 *
 * Intents open in it as they open in the product.
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
  const shown = pages ?? PLUGIN_PAGES.map((page) => {
    if (!pictures) {
      return page
    }
    return page.plugin === 'browser' ? browserPage({ pictures }) : page.plugin === 'mobile' ? mobilePage({ pictures }) : page
  })
  /** The plugins the specimen has turned off, as the user turns one off in Settings. */
  const off = reactive(new Set<string>())
  const enabled = (plugin: string) => !off.has(plugin)
  const backend = new GalleryPanel()
  const plugin = galleryBrowserPlugin(browser, backend)
  // The conversation's simulators, whose plugin opens and closes devices for the panel's `mobile` tabs.
  const mobile = galleryMobile(backend)
  // The browser tells the plugin of every change of its tabs, as the browser package does
  // (`crates/command-protocol/src/messages.rs`).
  watch(browser.listed, () => void plugin.tabsChanged())
  // The browser's tabs stand in the panel in its order, as the product's would: the user's with the tab the
  // user made for each, the agent's with the id the plugin gives them. A panel without the browser's page has none.
  if (shown.some((page) => page.plugin === 'browser')) {
    for (const tab of browser.listed.value.tabs) {
      const id = tab.createdBy.kind === 'user' ? `user-${tab.id}` : addedPanelTab(tab.id)
      backend.apply({ type: 'create', tab: { id, kind: 'browser', data: { url: tab.url, tab: tab.id, title: tab.title } } })
    }
  }
  const tabs = new PanelTabs(backend, (error) => {
    productWould(`Report the Refused Panel Change: ${error instanceof Error ? error.message : String(error)}`)
  })
  backend.removed = (tab) => {
    if (tab.kind === 'mobile') {
      mobile.removed(tab)
    } else if (tab.kind === 'browser') {
      void plugin.removed(tab)
    }
  }
  backend.changed = (revision) => tabs.noticed(revision)
  tabs.start()

  const history = ref<readonly string[]>(selection === null ? [] : [selection])
  const pinned = ref<PinnedTabs>({})
  const panel = computed<PanelState>(() => ({ history: history.value, tabs: tabs.tabs.value }))
  /** Whether the specimen's panel is open, which a tab its kind asks to show opens. */
  const open = ref(true)
  /**
   * The web browser's history of the panel, as the product's address keeps
   * it (`packages/web-ui/src/agent/panel-address.ts`): each move is a step,
   * and Back and Forward show the panel as it was there.
   */
  const steps = galleryHistory(() => ({ open: open.value, history: history.value, pinned: pinned.value }), (step) => {
    open.value = step.open
    history.value = step.history
    pinned.value = step.pinned
  })

  function select(id: string) {
    history.value = selectTab(history.value, id)
    steps.step()
  }
  /** Each tab's showings the panel applied, as the product keeps them beside the history. */
  let applied: AppliedShows = {}
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
  /** A new tab, after the others or at `options.index` among them, selected unless `options` says not; returns its id. */
  function add(kind: string, data: unknown, options: { select: boolean; index?: number } = { select: true }): string {
    const id = crypto.randomUUID()
    tabs.change({ type: 'create', tab: { id, kind, data }, index: options.index })
    if (options.select) {
      select(id)
    }
    return id
  }
  /** The number of the next browser tab `lost` names, past any the gallery's browser gives. */
  let lostTabs = 90
  /**
   * A browser tab on `url` whose browser tab the browser lost, as after a
   * restart: the panel the backend keeps has it already, as it had before,
   * and the page selects it once it arrives, so its content asks the plugin
   * to open it again, as a shown lost tab does.
   */
  function lost(url: string): string {
    const id = crypto.randomUUID()
    lostTabs += 1
    backend.apply({ type: 'create', tab: { id, kind: 'browser', data: { url, tab: `t${lostTabs}`, closed: true } } })
    select(id)
    return id
  }
  /** Moves the tab `id` to `index` among the others, where the user dragged it. */
  function move(id: string, index: number) {
    tabs.change({ type: 'move', id, index })
  }
  function update(id: string, data: unknown) {
    updatePanelTab(tabs, id, data)
  }
  function updatePinned(kind: string, data: unknown, how?: TabUpdate) {
    pinned.value = { ...pinned.value, [kind]: data }
    if (how?.replace) {
      steps.replace()
    } else {
      steps.step()
    }
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
    steps.replace()
  }
  /** Opens an intent as the product's work store does: what the panel already shows stays as it is. */
  function openIn(request: IntentRequest) {
    const opened = openIntent({ panel: panel.value, pinned: pinned.value }, shown, enabled, request)
    if (!opened) {
      return
    }
    pinned.value = opened.pinned
    if (opened.created) {
      tabs.change({ type: 'create', tab: opened.created })
    }
    open.value = true
    select(opened.selection)
  }

  const host = galleryPageHost({ browser: browserPlugin(browser, plugin), mobile: mobile.plugin }, {
    files,
    enabled,
    hostStarting: () => browser.hostStarting.value,
    intents: {
      open: (_conversation, request) => openIn(request),
      canOpen: (intent) => intentKind(shown, enabled, intent) !== null,
    },
    panel: {
      tabs: (_conversation, kind) => tabs.tabs.value.filter((tab) => tab.kind === kind).map((tab) => tab.data),
      select: (_conversation, _kind, id) => {
        open.value = true
        select(id)
      },
      canGo: (_conversation, direction) => steps.canGo(direction),
      go: (direction) => steps.go(direction),
    },
  })
  const kinds = useBoundKinds(() => shown.filter((page) => enabled(page.plugin)), host, () => CONVERSATION)
  /**
   * Turns `plugin` on or off while the panel is open, as the user does in
   * Settings: turned off, the backend closes its kinds' tabs in the panel and
   * releases what its packages hold on the Host, such as the browser, and the
   * shell disposes its stores; the other plugins' tabs and stores stay.
   */
  function setEnabled(pluginId: string, on: boolean) {
    if (on) {
      off.delete(pluginId)
      return
    }
    off.add(pluginId)
    const owned = new Set(shown.find((page) => page.plugin === pluginId)?.kinds?.map((kind) => kind.kind) ?? [])
    for (const tab of tabs.tabs.value.filter((entry) => owned.has(entry.kind))) {
      if (tab.kind === 'mobile') {
        // Its package closes the simulator, as the release of the plugin's Host resources does.
        mobile.removed(tab)
      }
      backend.apply({ type: 'remove', id: tab.id })
    }
    if (pluginId === 'browser') {
      browser.end()
    }
    disposePluginStores(host.pinia, pluginId)
  }
  /** What the panel shows, as its strip marks it. */
  const selected = computed(() => shownSelection(panel.value, kinds.value))

  /** A file pill, through the `edit` intent. */
  function selectEdit(edit: RequestEditSelection) {
    openIn({ intent: 'edit', payload: edit })
  }
  /** The panel as it started: every tab closed, the first selection again. */
  function reset() {
    closeTabs(tabs.tabs.value.map((tab) => tab.id))
    history.value = selection === null ? [] : [selection]
    pinned.value = {}
    steps.reset()
  }
  /**
   * The agent shows `tab` with `demi browser show`: the browser tells the
   * plugin, which carries the count into the panel tab, which the panel then
   * selects.
   */
  async function agentShows(tab: string) {
    browser.show(tab)
    await nextTick()
    await plugin.settled()
  }
  /** The agent opens a page, shown with `--show` or not: the browser tells the plugin, which adds the tab. */
  async function agentOpens(url: string, show: boolean) {
    browser.agentOpens(url, { show })
    await nextTick()
    await plugin.settled()
  }
  return {
    panel,
    pinned,
    open,
    kinds,
    selected,
    browser,
    mobile,
    agentShows,
    agentOpens,
    enabled,
    setEnabled,
    host,
    select,
    add,
    lost,
    move,
    update,
    updatePinned,
    closeTabs,
    openIn,
    selectEdit,
    reset,
  }
}

export type GalleryWork = ReturnType<typeof useGalleryWork>
