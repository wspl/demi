import { defineComponent, h, onBeforeUnmount, type PropType } from 'vue'
import { Globe, RotateCw } from '@lucide/vue'
import { ICON_PX, usePage, type KindTab, type PanelKind } from '@demicodes/plugin-sdk'
import BrowserTabContent from './BrowserTabContent.vue'
import { t } from '../locales/text'
import { BROWSER_TAB_KIND, type BrowserStore } from './store'
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
export function browserTabTitle(data: BrowserTabData, session: BrowserTabsController): string {
  const title = session.title(data) ?? data.title
  if (title) {
    return title
  }
  const url = URL.parse(data.url)
  return url ? url.host || url.href : data.url
}

/**
 * The `browser` tab kind over `store`. Its content watches the conversation's
 * browser while it is mounted and reaches the Host through it; its title, icon
 * and commands read it without watching; the panel sees only this
 * declaration.
 */
export function browserTabKind(store: BrowserStore): PanelKind<BrowserTabData> {
  const browser = (tab: KindTab) => tab.page.store(store).of(tab.conversation).tabs
  const content = defineComponent({
    props: {
      conversation: { type: String, required: true },
      tabId: { type: String, required: true },
      data: { type: Object as PropType<BrowserTabData>, required: true },
      shown: { type: Boolean, required: true },
    },
    emits: { update: (_data: BrowserTabData) => true },
    setup(props, { emit }) {
      // Watched while mounted: the conversation's browser is the most recent to keep.
      const watched = usePage().store(store).watch(props.conversation)
      onBeforeUnmount(() => watched.end())
      return () => h(BrowserTabContent, {
        conversation: props.conversation,
        session: watched.session.tabs,
        tabId: props.tabId,
        data: props.data,
        shown: props.shown,
        onUpdate: (data: BrowserTabData) => emit('update', data),
      })
    },
  })
  return {
    kind: BROWSER_TAB_KIND,
    schema: browserTabDataSchema,
    title: (data, tab) => browserTabTitle(data, browser(tab)),
    // The agent's showings, which the plugin carries from the tab list.
    shows: (data) => data.shows ?? 0,
    mark: BrowserTabMark,
    // The page's own icon, as a web browser's tab shows it; the mark only for a page without one.
    icon: (data, tab) => browser(tab).favicon(data),
    // The page loads: the strip shows it as a web browser's tab does.
    busy: (data, tab, id) => browser(tab).busy(id, data),
    content,
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
        run: () => browser(tab).reload(data),
      },
    ],
  }
}
