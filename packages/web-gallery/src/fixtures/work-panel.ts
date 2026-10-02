import { onBeforeUnmount, ref } from 'vue'
import { browserPage } from '@demicodes/plugin-browser'
import type { BrowserTabsApi } from '@demicodes/plugin-browser/live/tabs'
import {
  addTab,
  emptyPanelState,
  openIntent,
  removeTabs,
  selectInPanel,
  updateTab,
  type PanelState,
  type PinnedTabs,
} from '@demicodes/web-ui/agent/panel-tabs'
import type { CallEditSelection } from '@demicodes/web-ui/files/changes'
import type { IntentRequest } from '@demicodes/web-ui/plugins/intents'
import {
  bindPages,
  intentKind,
  type AnyPluginPage,
  type ConversationFileService,
} from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../generated/pages'
import { readGalleryEdit } from './blobs'
import { galleryBrowserTabs } from './live-browser'
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
 * One work panel's state the way the product's work store holds it
 * (`web-application.md` § Work panel): the selection and the user's tabs,
 * the pinned tabs' data, and the plugin pages' kinds bound to it over the
 * specimen's files and its own conversation browser. Intents open in it as
 * they open in the product.
 */
export function useGalleryWork(
  selection: string | null,
  { files, tabs = galleryBrowserTabs(), pictures, pages }: {
    files: ConversationFileService
    tabs?: BrowserTabsApi
    /** Whether the web browser decodes the pictures; by default it asks the web browser, as the product does. */
    pictures?: () => Promise<boolean>
    /** The plugin pages whose kinds the panel shows; every one the product shows by default. */
    pages?: readonly AnyPluginPage[]
  },
) {
  const panel = ref<PanelState>({ ...emptyPanelState(), selection })
  const pinned = ref<PinnedTabs>({})
  // The gallery's pages, with the browser's own made for a specimen that says how the pictures decode.
  const shown = pages ?? PLUGIN_PAGES.map((page) => (page.plugin === 'browser' && pictures ? browserPage({ pictures }) : page))
  const enabled = () => true

  function openIn(request: IntentRequest) {
    const opened = openIntent({ state: panel.value, pinned: pinned.value }, shown, enabled, request)
    if (opened) {
      panel.value = opened.state
      pinned.value = opened.pinned
    }
  }

  const host = galleryPageHost({ browser: browserPlugin(tabs) }, {
    files,
    intents: {
      open: (_conversation, request) => openIn(request),
      canOpen: (intent) => intentKind(shown, enabled, intent) !== null,
    },
    panel: {
      tabs: (_conversation, kind) => panel.value.tabs.filter((tab) => tab.kind === kind).map((tab) => tab.data),
      add: (_conversation, kind, data, options = { select: false }) => {
        panel.value = addTab(panel.value, { kind, data }, options).state
      },
    },
  })
  const bound = bindPages(shown, host, CONVERSATION)
  onBeforeUnmount(() => bound.dispose())
  const kinds = bound.kinds

  function select(next: string | null) {
    panel.value = selectInPanel(panel.value, next)
  }
  function add(kind: string, data: unknown) {
    panel.value = addTab(panel.value, { kind, data }, { select: true }).state
  }
  function update(id: string, data: unknown) {
    panel.value = updateTab(panel.value, id, data)
  }
  function updatePinned(kind: string, data: unknown) {
    pinned.value = { ...pinned.value, [kind]: data }
  }
  /** A closed tab goes at once; its kind then does what a closed tab of it needs. */
  function closeTabs(ids: string[]) {
    const closing = panel.value.tabs.filter((tab) => ids.includes(tab.id))
    panel.value = removeTabs(panel.value, ids)
    for (const tab of closing) {
      const kind = kinds.find((candidate) => candidate.kind === tab.kind)
      const parsed = kind?.schema.safeParse(tab.data)
      if (kind?.removed && parsed?.success) {
        kind.removed(parsed.data)
      }
    }
  }
  /** A tool call's file pill, through the `edit` intent. */
  function selectEdit(edit: CallEditSelection) {
    openIn({ intent: 'edit', payload: edit })
  }
  function reset() {
    panel.value = { ...emptyPanelState(), selection }
    pinned.value = {}
  }
  return {
    panel,
    pinned,
    kinds,
    tabs,
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
