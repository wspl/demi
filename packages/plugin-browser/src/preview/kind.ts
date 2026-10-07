import { defineComponent, h } from 'vue'
import { Bot, Globe, RotateCw } from '@lucide/vue'
import { GlobePlus, ICON_PX, type PanelKind, type TabCommand } from '@demicodes/plugin-sdk'
import type { BrowserPanel } from '../panel'
import PreviewTabContent from './PreviewTabContent.vue'
import { previewTabDataSchema, type PreviewTabData } from './tabs'

/** A page without an icon of its own, as a web browser marks one. */
const PreviewTabMark = defineComponent({
  setup: () => () => h(Globe, { size: ICON_PX.markIn28 }),
})

/**
 * A tab is named by its page's title, else by its address's host; a new
 * tab is New Tab, as in any browser.
 */
function previewTabTitle(data: PreviewTabData): string {
  if (data.url === '') {
    return 'New Tab'
  }
  if (data.title) {
    return data.title
  }
  const url = URL.parse(data.url)
  return url ? url.host || url.href : data.url
}

/** Open in Agent's Browser, unavailable with why while the tab has no page or the Host no browser. */
function agentsBrowserCommand(session: BrowserPanel, id: string): TabCommand {
  const shown = session.preview.tab(id)?.view.page
  const reason = shown ? session.browser.unavailable.value : 'The tab has no page to open yet.'
  return {
    label: 'Open in Agent’s Browser',
    icon: Bot,
    disabled: reason !== null,
    ...(reason ? { disabledReason: reason } : {}),
    run: () => void session.preview.toAgent(id),
  }
}

/**
 * The `preview` tab kind, a tab of the user's browser (`preview.md` § What
 * the user sees): a web preview of a page of the conversation's Host,
 * rendered by the user's own browser. The strip's + opens one, with an
 * empty address bar, as a browser's new tab does.
 */
export const previewTabKind: PanelKind<PreviewTabData, BrowserPanel> = {
  kind: 'preview',
  schema: previewTabDataSchema,
  title: (data) => previewTabTitle(data),
  openedBy: (data) => data.openedBy,
  mark: PreviewTabMark,
  // The page's own icon, as a web browser's tab shows it; the mark for a page without one.
  icon: (_data, tab, id) => tab.session.preview.tab(id)?.view.icon ?? null,
  busy: (_data, tab, id) => tab.session.preview.tab(id)?.view.loading ?? false,
  content: PreviewTabContent,
  commands: (data, tab, id) => [
    {
      label: 'Reload',
      icon: RotateCw,
      disabled: data.url === '',
      run: () => tab.session.preview.tab(id)?.history('reload'),
    },
    // The page in the agent's browser, beside this tab, with its state (`preview.md` § What the user sees).
    agentsBrowserCommand(tab.session, id),
  ],
  // A page the agent presented opens with its state (`preview.md` § Presenting a page).
  intents: {
    page: (payload) => ({ url: payload.url, ...(payload.title ? { title: payload.title } : {}), from: payload.tab }),
  },
  duplicate: (data) => (data.title ? { url: data.url, title: data.title } : { url: data.url }),
  create: {
    label: 'New Tab',
    icon: GlobePlus,
    data: () => ({ url: '' }),
    unavailable: (tab) => tab.session.preview.unavailable.value,
  },
}
