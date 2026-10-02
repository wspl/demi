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

/**
 * A tab is named by its page once the browser shows the address the tab
 * asks for, and by that address until then: a new tab is `about:blank` from
 * the start, and a tab its user sent elsewhere is named after where it goes
 * at once, never after the page it leaves.
 */
function browserTabTitle(controller: BrowserTabsController, data: BrowserTabData): string {
  const known = controller.listed(data.tab)
  if (known?.title && known.url === data.url) {
    return known.title
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
  create: {
    label: "New tab in the conversation's browser",
    icon: GlobePlus,
    data: () => ({ url: NEW_TAB_URL }),
  },
}
