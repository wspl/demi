import { defineComponent, h } from 'vue'
import { Globe, RotateCw } from '@lucide/vue'
import { GlobePlus, ICON_PX, type PanelKind } from '@demicodes/plugin-sdk'
import BrowserTabContent from './BrowserTabContent.vue'
import {
  NEW_TAB_URL,
  browserTabDataSchema,
  type BrowserTabData,
  type BrowserTabsController,
} from './tabs'

const BrowserTabMark = defineComponent({
  setup: () => () => h(Globe, { size: ICON_PX.markIn28 }),
})

/**
 * A tab is named by its page's title: as the browser reports it, so a tab
 * opened in the background takes its title as soon as its page has one, as
 * in any browser; else as the address bar last showed it, which the tab
 * keeps, so the strip names it while nothing reports it; until it has one,
 * by its address. A new tab is New Tab, as in any browser, and a tab its
 * user sent elsewhere is named after where it goes at once, never after the
 * page it leaves.
 */
function browserTabTitle(data: BrowserTabData, session: BrowserTabsController): string {
  if (data.url === NEW_TAB_URL) {
    return 'New Tab'
  }
  const title = session.title(data) ?? data.title
  if (title) {
    return title
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
  title: (data, tab) => browserTabTitle(data, tab.session),
  // The agent's showings, which the plugin carries from the tab list (`live-view.md` § Showing a tab).
  shows: (data) => data.shows ?? 0,
  mark: BrowserTabMark,
  // The page's own icon, as a web browser's tab shows it; the mark only for a page without one.
  icon: (data, tab) => tab.session.favicon(data),
  // The page loads: the strip shows it as a web browser's tab does.
  busy: (data, tab, id) => tab.session.busy(id, data),
  content: BrowserTabContent,
  // A web browser's tab menu: Reload the page, or open the address again in a tab of its own.
  commands: (data, tab) => [
    {
      label: 'Reload',
      icon: RotateCw,
      disabled: data.tab === undefined || data.closed === true,
      run: () => tab.session.reload(data),
    },
  ],
  duplicate: (data) => (data.title ? { url: data.url, title: data.title } : { url: data.url }),
  create: {
    label: "New Tab in the Conversation's Browser",
    icon: GlobePlus,
    data: () => ({ url: NEW_TAB_URL }),
    unavailable: (tab) => tab.session.unavailable.value,
  },
}
