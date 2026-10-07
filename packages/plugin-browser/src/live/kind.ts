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
 * A tab is named by its page's title as the address bar last showed it,
 * which the tab keeps, so the strip names it while no view shows it; until
 * it has one, by its address: a new tab is `about:blank` from the start, and
 * a tab its user sent elsewhere is named after where it goes at once, never
 * after the page it leaves.
 */
function browserTabTitle(data: BrowserTabData): string {
  if (data.title) {
    return data.title
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
  title: (data) => browserTabTitle(data),
  // The agent's showings, which the plugin carries from the tab list (`live-view.md` § Showing a tab).
  shows: (data) => data.shows ?? 0,
  mark: BrowserTabMark,
  // The page loads: the strip shows it as a web browser's tab does.
  busy: (data, tab, id) => tab.session.busy(id, data),
  content: BrowserTabContent,
  create: {
    label: "New Tab in the Conversation's Browser",
    icon: GlobePlus,
    data: () => ({ url: NEW_TAB_URL }),
    unavailable: (tab) => tab.session.unavailable.value,
  },
}
