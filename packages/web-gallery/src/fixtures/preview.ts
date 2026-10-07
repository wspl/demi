/**
 * Tabs of the user's browser without a Host or a preview domain
 * (`preview.md` § What the user sees): the gallery serves each address as a
 * page of its own, which tells the tab its title and icon as a preview's
 * runtime does, keeps each tab's history, and answers the plugin's opening.
 * A page's links act as a preview's do: within the tab, in a new tab beside
 * it, and on to another site.
 */
import { shallowRef, type ShallowRef } from 'vue'
import type { PageStorage, PreviewOpened } from '@demicodes/plugin-browser/generated/plugin'
import type { NavigationType, RelayBinding, TabPage } from '@demicodes/plugin-browser/preview/relay'
import type { PreviewDriver, PreviewTab } from '@demicodes/plugin-browser/preview/tabs'
import type { PreviewPlace } from '@demicodes/web-ui/plugins/page'

/** How long the gallery's Host takes to answer a page, as a near one does. */
const BEAT_MS = 600

/** Each site's icon, drawn once. */
const siteIcons = new Map<string, string>()

/**
 * A fixture site's icon, 32 pixels square: the first letter of its host on a
 * color of its own. `plain.test` has none, so its tabs show the generic
 * mark, as a site without an icon does.
 */
function siteIcon(url: string): string | undefined {
  const parsed = URL.parse(url)
  if (!parsed || (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') || parsed.host === 'plain.test') {
    return undefined
  }
  const known = siteIcons.get(parsed.host)
  if (known) {
    return known
  }
  const canvas = document.createElement('canvas')
  canvas.width = 32
  canvas.height = 32
  const context = canvas.getContext('2d')
  if (!context) {
    return undefined
  }
  const hue = [...parsed.host].reduce((sum, letter) => sum + letter.charCodeAt(0), 0) % 360
  context.fillStyle = `hsl(${hue} 60% 45%)`
  context.beginPath()
  context.roundRect(0, 0, 32, 32, 7)
  context.fill()
  context.fillStyle = '#fff'
  context.font = 'bold 20px sans-serif'
  context.textAlign = 'center'
  context.textBaseline = 'middle'
  context.fillText(parsed.host.charAt(0).toUpperCase(), 16, 17)
  const icon = canvas.toDataURL('image/png')
  siteIcons.set(parsed.host, icon)
  return icon
}

/** What the gallery's pages say they are, by path. */
const PAGES: Record<string, { title: string; text: string }> = {
  '/': { title: 'Storefront', text: 'The home page of the shop the agent is building, as its development server serves it.' },
  '/orders': { title: 'Orders', text: 'Three orders wait to ship. The page is the Host’s, rendered here by your own browser.' },
  '/docs': { title: 'Docs', text: 'Reference pages open in a tab of their own.' },
}

/** The page a gallery address shows: its own text, links and a script that hands its clicks to the tab. */
function pageHtml(url: URL): string {
  const known = PAGES[url.pathname]
  const title = known ? `${known.title} — ${url.host}` : url.host
  const text = known?.text ?? 'A page of the Host, rendered by your browser.'
  const link = (href: string, label: string, target = '') =>
    `<a href="${new URL(href, url).href}"${target ? ` target="${target}"` : ''}>${label}</a>`
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title>
<style>body{font:15px system-ui,sans-serif;margin:32px;color:#1f2933}h1{font-size:22px;margin:0 0 12px}nav{display:flex;gap:16px;margin-top:20px}a{color:#2563eb}</style>
</head><body><h1>${known?.title ?? url.host}</h1><p>${text}</p>
<nav>${link('/', 'Home')}${link('/orders', 'Orders')}${link('/docs', 'Docs in a new window', '_blank')}${link('https://plain.test/', 'Another site')}</nav>
<script>addEventListener('message', (event) => { if (event.data?.galleryPreview && event.data.go) location.replace(event.data.go); });
document.addEventListener('click', (event) => {
  const anchor = event.target.closest('a');
  if (!anchor) return;
  event.preventDefault();
  parent.postMessage({ galleryPreview: true, url: anchor.getAttribute('href'), window: anchor.target === '_blank' || event.metaKey || event.ctrlKey }, '*');
});</script></body></html>`
}

/** A tab's pages, as a browser's history keeps them. */
interface History {
  entries: string[]
  /** Each entry's key, as the Navigation API names an entry. */
  keys: string[]
  index: number
  /** How the tab got to the page it loads, which the page reports once loaded. */
  move: NavigationType
  /** The page each entry shows, while the tab lives. */
  documents: string[]
}

export interface GalleryPreview {
  driver(): PreviewDriver
  /** Where the gallery's previews live. */
  place(): PreviewPlace
  /** Why this page cannot show previews, as the specimen says; null when it can. */
  readonly unsupported: ShallowRef<string | null>
  /** The plugin's `preview_open`: the address's top-level label, after a beat. */
  open(url: string): Promise<PreviewOpened>
  /** The next opening fails, as a Host that cannot reach the address answers. */
  failNext(reason: string): void
  /** The agent's browser whose tabs' pages a page state takes, by tab. */
  takeFrom(pages: (tab: string) => { url: string; title: string } | null): void
  /** Openings wait here until `release`, as a far Host takes its time. */
  hold(): void
  release(): void
  readonly held: ShallowRef<boolean>
}

/** A site's state as the gallery's pages keep it once signed in: what a page state moves. */
function signedIn(origin: string): PageStorage {
  return {
    origin,
    local: [{ key: 'session', value: 'signed-in' }],
    session: [],
    databases: [],
    skipped: [],
  }
}

/** The gallery's previews, whose pages are its own. */
export function galleryPreview(options: { unsupported?: string } = {}): GalleryPreview {
  // The pages of the agent's tabs, which Open in Your Browser and a presented page's Open take.
  let agentPages: (tab: string) => { url: string; title: string } | null = () => null
  const histories = new Map<string, History>()
  const unsupported = shallowRef<string | null>(options.unsupported ?? null)
  const held = shallowRef(false)
  let waiting: (() => void)[] = []
  let failure: string | null = null
  let labels = 0

  const wait = async () => {
    await new Promise((resolve) => setTimeout(resolve, BEAT_MS))
    if (held.value) {
      await new Promise<void>((resolve) => waiting.push(resolve))
    }
  }

  /** The page `url` shows, made once per tab entry. */
  function show(history: History, url: string): string {
    const document = URL.createObjectURL(new Blob([pageHtml(new URL(url))], { type: 'text/html' }))
    history.documents.push(document)
    return document
  }

  /** What the tab's top page says once it loaded: how its history moved there, and what it is. */
  function report(tab: PreviewTab, history: History): void {
    const url = history.entries[history.index]!
    const known = PAGES[new URL(url).pathname]
    const page: TabPage = {
      url,
      title: known ? `${known.title} — ${new URL(url).host}` : new URL(url).host,
      icon: siteIcon(url) ?? '',
    }
    tab.report({ type: 'entry', key: history.keys[history.index]!, navigationType: history.move })
    tab.report({ type: 'page', page })
  }

  /** A binding of the gallery's page that opens a window, as a preview's runtime would ask for it. */
  function opener(tab: PreviewTab, url: string): RelayBinding {
    const origin = new URL(url).origin
    return {
      port: new MessageChannel().port1,
      origin: 'https://gallery0--gallery.demi-preview.dev',
      label: 'gallery',
      environment: { origin, top: origin, cross: false },
      tab,
      source: tab.frameWindow(),
    }
  }

  const driver: PreviewDriver = {
    // The gallery's pages load at once, with no boot page before them.
    boots: false,
    // The page loaded: it tells its tab what it is.
    loaded(tab) {
      const history = histories.get(tab.id)
      if (history && history.index >= 0) {
        report(tab, history)
      }
    },
    register(tab) {
      const history: History = { entries: [], keys: [], index: -1, move: 'push', documents: [] }
      histories.set(tab.id, history)
      // A page's clicks, which it hands its tab as a preview's runtime does.
      const clicked = (event: MessageEvent) => {
        const data = event.data as { galleryPreview?: boolean; url?: string; window?: boolean } | null
        if (!data?.galleryPreview || !data.url || event.source !== tab.frameWindow()) {
          return
        }
        const current = history.entries[history.index] ?? data.url
        if (data.window) {
          tab.openWindow(opener(tab, current), 1, { url: data.url, initiator: { origin: new URL(current).origin, top: new URL(current).origin, cross: false } })
        } else {
          tab.navigate({ url: data.url, initiator: { origin: new URL(current).origin, top: new URL(current).origin, cross: false } })
        }
      }
      window.addEventListener('message', clicked)
      return () => {
        window.removeEventListener('message', clicked)
        for (const document of history.documents) {
          URL.revokeObjectURL(document)
        }
        histories.delete(tab.id)
      }
    },
    async boot(tab, _place, _opened, navigation) {
      const history = histories.get(tab.id)!
      history.entries = [...history.entries.slice(0, history.index + 1), navigation.url]
      history.keys = [...history.keys.slice(0, history.index + 1), crypto.randomUUID()]
      history.index = history.entries.length - 1
      history.move = 'push'
      return show(history, navigation.url)
    },
    command(tab, command) {
      const history = histories.get(tab.id)
      const frame = tab.frameWindow()
      if (!history || !frame || history.index < 0) {
        return false
      }
      if (command === 'stop') {
        return true
      }
      const index = history.index + (command === 'back' ? -1 : command === 'forward' ? 1 : 0)
      if (index < 0 || index >= history.entries.length) {
        return true
      }
      history.index = index
      history.move = command === 'reload' ? 'reload' : 'traverse'
      tab.report({ type: 'leaving' })
      // The page goes there itself, as a page's own history move does.
      frame.postMessage({ galleryPreview: true, go: show(history, history.entries[index]!) }, '*')
      return true
    },
    icon: async (_tab, _place, page) => page.icon || null,
    // The agent's tab signed in to the gallery's site: its state moves as a
    // page state does, a beat later, and the page opens with it.
    async takeState(_tab, _place, from) {
      await wait()
      const page = agentPages(from)
      if (!page) {
        throw new Error('The agent’s tab is gone.')
      }
      return { ...page, mobile: false, storage: signedIn(new URL(page.url).origin), tooLarge: false }
    },
    writeState: async () => [],
    async readState(tab) {
      const history = histories.get(tab.id)
      const url = history?.entries[history.index]
      return url ? signedIn(new URL(url).origin) : null
    },
    origins(tab) {
      const history = histories.get(tab.id)
      const url = history?.entries[history.index]
      return url ? [new URL(url).origin] : []
    },
    keepState: async () => {},
  }

  return {
    driver: () => driver,
    place: () => ({
      scheme: 'https',
      domain: 'demi-preview.dev',
      namespace: 'gallery0',
      host: 'gallery-device',
      runtime: { release: '0.0.0', url: '/runtime/0.0.0.js' },
    }),
    unsupported,
    async open(url) {
      await wait()
      if (failure) {
        const reason = failure
        failure = null
        throw new Error(reason)
      }
      const origin = new URL(url).origin
      const label = `gallery${String(labels++).padStart(9, '0')}`
      return {
        label,
        environment: { origin, top: origin, cross: false },
        origin: `https://gallery0--${label}.demi-preview.dev`,
      }
    },
    failNext(reason) {
      failure = reason
    },
    takeFrom(pages) {
      agentPages = pages
    },
    hold() {
      held.value = true
    },
    release() {
      held.value = false
      for (const resolve of waiting) {
        resolve()
      }
      waiting = []
    },
    held,
  }
}
