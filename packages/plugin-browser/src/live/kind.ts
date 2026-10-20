import { defineComponent, h } from 'vue'
import { Globe, RotateCw } from '@lucide/vue'
import { ICON_PX, type PanelKind } from '@demicodes/plugin-sdk'
import BrowserTabContent from './BrowserTabContent.vue'
import { t } from '../locales/text'
import { browserTabDataSchema, type BrowserTabData, type BrowserTabsController } from './tabs'

const BrowserTabMark = defineComponent({
  setup: () => () => h(Globe, { size: ICON_PX.markIn28 }),
})

/**
 * A tab is named by its page's title, in this order:
 *
 * 1. The title as the browser reports it, so a tab opened in the
 *    background takes its title as soon as its page has one, as in any
 *    browser.
 * 2. Else the title as the address bar last showed it, which the tab
 *    keeps, so the strip names it while nothing reports it.
 * 3. Until it has one, its address.
 *
 * A tab its user sent elsewhere is named after where it goes at once,
 * never after the page it leaves.
 */
function browserTabTitle(data: BrowserTabData, session: BrowserTabsController): string {
  const title = session.title(data) ?? data.title
  if (title) {
    return title
  }
  const url = URL.parse(data.url)
  return url ? url.host || url.href : data.url
}

/**
 * The `browser` tab kind. Its content reaches the conversation's browser
 * through the page's panel session; the panel sees only this declaration.
 */
export const browserTabKind: PanelKind<BrowserTabData, BrowserTabsController> = {
  kind: 'browser',
  schema: browserTabDataSchema,
  title: (data, tab) => browserTabTitle(data, tab.session),
  // The agent's showings, which the plugin carries from the tab list.
  shows: (data) => data.shows ?? 0,
  mark: BrowserTabMark,
  // The page's own icon, as a web browser's tab shows it; the mark only for a page without one.
  icon: (data, tab) => tab.session.favicon(data),
  // The page loads: the strip shows it as a web browser's tab does.
  busy: (data, tab, id) => tab.session.busy(id, data),
  content: BrowserTabContent,
  // A link's Open in Work Panel: a new tab on its page, which the shown content asks the plugin to open (`bind`).
  intents: {
    web: (payload) => ({ url: payload.url }),
  },
  // A web browser's tab menu: Reload the page.
  commands: (data, tab) => [
    {
      label: t('menu.reload'),
      icon: RotateCw,
      disabled: data.tab === undefined || data.closed === true,
      run: () => tab.session.reload(data),
    },
  ],
}
