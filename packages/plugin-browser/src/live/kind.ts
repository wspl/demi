import { defineComponent, h } from 'vue'
import { MonitorDot } from '@lucide/vue'
import { GlobePlus, ICON_PX, type PanelKind } from '@demicodes/plugin-sdk'
import BrowserTabContent from './BrowserTabContent.vue'
import {
  NEW_TAB_URL,
  browserTabDataSchema,
  type BrowserTabData,
  type BrowserTabsController,
} from './tabs'

const BrowserTabMark = defineComponent({
  setup: () => () => h(MonitorDot, { size: ICON_PX.markIn28 }),
})

/** A tab is named by its page once the conversation browser has one, and by its address until then. */
function browserTabTitle(controller: BrowserTabsController, data: BrowserTabData): string {
  const known = controller.list.value?.tabs.find((tab) => tab.id === data.tab)
  if (known?.title) {
    return known.title
  }
  if (data.url === NEW_TAB_URL) {
    return 'New tab'
  }
  const url = URL.parse(data.url)
  return url ? url.host || url.href : data.url
}

/**
 * The `browser` tab kind (`live-view.md` § A browser tab in the panel). Its
 * content reaches the conversation's browser through the page's panel
 * session; the panel sees only this declaration.
 */
export const browserTabKind: PanelKind<BrowserTabData, BrowserTabsController> = {
  kind: 'browser',
  schema: browserTabDataSchema,
  title: (data, tab) => browserTabTitle(tab.session, data),
  mark: BrowserTabMark,
  content: BrowserTabContent,
  removed(data, tab) {
    if (data.tab === undefined) {
      return
    }
    // The panel tab is gone either way; the controller says what a refused close leaves.
    tab.session.close(data.tab).catch(() => {})
  },
  create: {
    label: "New tab in the conversation's browser",
    icon: GlobePlus,
    data: () => ({ url: NEW_TAB_URL }),
  },
}
