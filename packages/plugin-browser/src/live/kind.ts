import { defineComponent, h } from 'vue'
import { Bot, Globe, RotateCw } from '@lucide/vue'
import { ICON_PX, type PanelKind, type TabCommand } from '@demicodes/plugin-sdk'
import type { BrowserPanel } from '../panel'
import BrowserTabContent from './BrowserTabContent.vue'
import {
  NEW_TAB_URL,
  browserTabDataSchema,
  type BrowserTabData,
  type BrowserTabsController,
} from './tabs'

/**
 * The agent's browser's one fixed icon, which tells its tabs from the tabs of
 * the user's browser, which show their pages' own (`preview.md` § What the
 * user sees).
 */
const BrowserTabMark = defineComponent({
  setup: () => () => h(Bot, { size: ICON_PX.markIn28 }),
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
export function browserTabTitle(data: BrowserTabData, session: BrowserTabsController): string {
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

/** Open in Your Browser, unavailable with why where previews cannot run or the tab has no web page. */
function yourBrowserCommand(session: BrowserPanel, data: BrowserTabData, id: string): TabCommand {
  const address = session.browser.address(data)
  const page = data.tab !== undefined && data.closed !== true && /^https?:/.test(address)
  const reason = session.preview.unavailable.value ?? (page ? null : 'The tab has no web page to open yet.')
  return {
    label: 'Open in Your Browser',
    icon: Globe,
    disabled: reason !== null,
    ...(reason ? { disabledReason: reason } : {}),
    run: () => {
      const title = session.browser.title(data) ?? data.title
      session.preview.fromAgent(id, { ...data, url: address, ...(title ? { title } : {}) })
    },
  }
}

/**
 * The `browser` tab kind (`live-view.md` § A browser tab in the panel). Its
 * content reaches the conversation's browser through the page's panel
 * session; the panel sees only this declaration.
 */
export const browserTabKind: PanelKind<BrowserTabData, BrowserPanel> = {
  kind: 'browser',
  schema: browserTabDataSchema,
  title: (data, tab) => browserTabTitle(data, tab.session.browser),
  // The agent's showings, which the plugin carries from the tab list (`live-view.md` § Showing a tab).
  shows: (data) => data.shows ?? 0,
  // Which tab opened it, so the panel places the next tab its opener opens behind it.
  openedBy: (data) => data.openedBy,
  mark: BrowserTabMark,
  // The page loads: the strip shows it as a web browser's tab does.
  busy: (data, tab, id) => tab.session.browser.busy(id, data),
  content: BrowserTabContent,
  // A web browser's tab menu: Reload the page, or open the address again in a tab of its own.
  commands: (data, tab, id) => [
    {
      label: 'Reload',
      icon: RotateCw,
      disabled: data.tab === undefined || data.closed === true,
      run: () => tab.session.browser.reload(data),
    },
    // The page in the user's own browser, beside this tab, with its state (`preview.md` § What the user sees).
    yourBrowserCommand(tab.session, data, id),
  ],
  duplicate: (data) => (data.title ? { url: data.url, title: data.title } : { url: data.url }),
}
