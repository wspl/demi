import { expect, test } from 'bun:test'
import { computed } from 'vue'
import type { PreviewPlace } from '@demicodes/plugin-sdk'
import type { PreviewOpened } from '../../generated/plugin'
import { PreviewConnection } from '../connection'
import { PreviewTabs, TabHistory, type PreviewTabData } from '../tabs'

// A tab's Back and Forward (`preview.md` § What the user sees) count the
// pages of every origin the tab showed, from the moves its top documents
// report; a page's own Navigation API sees only its origin's. The tab shows
// the page loading from the click of Back, Forward or Reload. No DOM; a few
// milliseconds.

test('Back and Forward count the pages of every origin, as a browser’s do', () => {
  const history = new TabHistory()
  const state = () => [history.canGoBack, history.canGoForward]
  // The user opens the app; a link goes on to another origin.
  history.moved('app', 'push')
  expect(state()).toEqual([false, false])
  history.moved('docs', 'push')
  expect(state()).toEqual([true, false])
  // Back returns to the app's entry: the docs page is ahead of it.
  history.moved('app', 'traverse')
  expect(state()).toEqual([false, true])
  history.moved('docs', 'traverse')
  expect(state()).toEqual([true, false])
  // A page that replaces its entry, or reloads, keeps its place.
  history.moved('docs-replaced', 'replace')
  history.moved('docs-replaced', 'reload')
  expect(state()).toEqual([true, false])
  // A new page from an earlier entry drops the ones ahead of it.
  history.moved('app', 'traverse')
  history.moved('checkout', 'push')
  expect(state()).toEqual([true, false])
  history.moved('app', 'traverse')
  expect(state()).toEqual([false, true])
  history.moved('checkout', 'traverse')
  expect(state()).toEqual([true, false])
})

test('a first move of any kind, and a move to an entry the tab never met, are new pages', () => {
  const history = new TabHistory()
  // The first page the tab shows, which its boot page reached by replacing itself.
  history.moved('first', 'replace')
  expect([history.canGoBack, history.canGoForward]).toEqual([false, false])
  history.moved('unknown', 'traverse')
  expect([history.canGoBack, history.canGoForward]).toEqual([true, false])
})

/** A tab of the user's browser showing a page whose runtime takes the bar's commands. */
async function shownTab() {
  const commands: string[] = []
  const place: PreviewPlace = {
    scheme: 'https',
    domain: 'demi-preview.dev',
    namespace: 'k3f9x2ab',
    host: 'host-1',
    runtime: { release: '0.1.20', url: '/runtime/0.1.20.js' },
  }
  const opened: PreviewOpened = {
    label: 'selbnt2qp6d94in3',
    environment: { origin: 'http://localhost:5173', top: 'http://localhost', cross: false },
    origin: 'https://k3f9x2ab--selbnt2qp6d94in3.demi-preview.dev',
  }
  const tabs = new PreviewTabs(
    {
      place: computed(() => place),
      connection: new PreviewConnection(() => ({ send() {}, close() {} })),
      hostStarting: () => false,
      open: async () => opened,
      add: () => {},
      addAgentTab: () => {},
      notify: () => {},
    },
    {
      boots: true,
      register: () => () => {},
      boot: async () => `${opened.origin}/__demi/v1/boot.html`,
      command: (_tab, command) => {
        commands.push(command)
        return true
      },
      icon: async () => null,
      takeState: async () => {
        throw new Error('unused')
      },
      writeState: async () => [],
      readState: async () => null,
      origins: () => [],
      keepState: async () => {},
    },
    () => null,
  )
  let data: PreviewTabData = { url: 'http://localhost:5173/' }
  const tab = tabs.attach('preview-1', { frame: () => null, data: () => data, update: (next) => (data = next), close: () => {} })
  for (let turn = 0; turn < 10; turn++) {
    await Promise.resolve()
  }
  // The boot page loads, then the page it opens, which reports itself; a link went on to a second page.
  tab.loaded()
  tab.report({ type: 'entry', key: 'one', navigationType: 'push' })
  tab.loaded()
  tab.report({ type: 'leaving' })
  tab.report({ type: 'entry', key: 'two', navigationType: 'push' })
  tab.loaded()
  return { tab, commands }
}

test('Back, Forward and Reload show the tab loading from the click, until the page loads or moves within itself', async () => {
  // Over a far relay the page's next document answers a second or more later; the bar shows Stop, the
  // strip its spinner and the progress line at the click, as a browser's do.
  const { tab, commands } = await shownTab()
  expect(tab.view.loading).toBe(false)
  tab.history('reload')
  expect([commands.at(-1), tab.view.loading]).toEqual(['reload', true])
  // The document goes when its answer came, and the next one loads.
  tab.report({ type: 'leaving' })
  tab.report({ type: 'entry', key: 'two-reloaded', navigationType: 'reload' })
  expect(tab.view.loading).toBe(true)
  tab.loaded()
  expect(tab.view.loading).toBe(false)
  // Back to an entry of the same document: its move ends the loading, since no document loads.
  tab.history('back')
  expect([commands.at(-1), tab.view.loading]).toEqual(['back', true])
  tab.report({ type: 'entry', key: 'one', navigationType: 'traverse' })
  expect(tab.view.loading).toBe(false)
  // Forward to another document loads it.
  tab.history('forward')
  expect(tab.view.loading).toBe(true)
  tab.report({ type: 'leaving' })
  tab.report({ type: 'entry', key: 'two-reloaded', navigationType: 'traverse' })
  tab.loaded()
  expect(tab.view.loading).toBe(false)
})
