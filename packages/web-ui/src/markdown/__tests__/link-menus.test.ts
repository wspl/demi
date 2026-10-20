import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A link to an object has that object's own menu: a file's, a commit's or an
// agent's, Open first, then the message's own group below, while a web link has
// the entries a browser's menu gives it and Open in Work Panel. A reply renders
// on a page of happy-dom through the product's StreamedMarkdown, inside a
// message that declares its menu, and the page's root handler builds what a
// right-click opens; what a row copies is read off a stubbed clipboard. Tens of
// ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, withDirectives } = await import('vue')
const menus = await import('../../ui/context-menu')
const { provideMessageFiles } = await import('../message-files')
const { provideMessageAgents } = await import('../../agent/agent-links')
const { provideMediaViewer } = await import('../../files/media-viewer')
const { default: StreamedMarkdown } = await import('../../ui/StreamedMarkdown.vue')
type MessageFiles = import('../types').MessageFiles
type MessageAgents = import('../../agent/agent-links').MessageAgents
type AgentRun = import('../../agent/subagents').AgentRun
type Group = readonly import('../../ui/context-menu').ContextMenuEntry[]

/** What the clipboard was given, as text, each copy in turn. */
const clipboard: string[] = []
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: {
    writeText: async (text: string) => void clipboard.push(text),
    write: async (items: ClipboardItem[]) => {
      for (const item of items)
        clipboard.push(await (await item.getType('text/plain')).text())
    },
  },
})

let teardown: (() => void)[] = []
afterEach(() => {
  for (const undo of teardown.reverse())
    undo()
  teardown = []
  menus.closeContextMenu()
  clipboard.length = 0
  document.body.innerHTML = ''
})

const FULL = '3f2a9c1e5b7d4a6f8e0c2b4d6a8f0e1c3b5d7a9f'
const done: string[] = []

const files: MessageFiles = {
  cwd: '/work',
  contents: {
    url: (path, options) => `/raw?path=${encodeURIComponent(path)}${options?.download ? '&download' : ''}`,
    describe: () => new Promise(() => {}),
    showDescription: () => { throw new Error('No file is shown') },
  },
  open: ({ path }) => void done.push(`open ${path}`),
  showInFiles: ({ path }) => void done.push(`show in files ${path}`),
  openCommit: ({ commit }) => void done.push(`open commit ${commit}`),
  commitHash: async ({ commit }) => {
    if (!FULL.startsWith(commit))
      throw new Error(`No commit ${commit}`)
    return FULL
  },
}

/** A round of an agent in `phase`. */
const run = (phase: AgentRun['phase']): AgentRun => ({
  profile: null,
  model: { providerId: 'p', provider: 'Anthropic', model: 'Claude', effort: null, fast: false },
  round: 1,
  forkedFrom: null,
  phase,
  startedAt: '2026-10-18T10:00:00Z',
  cwd: '/work',
})

/** Agent 4 runs; agent 5 finished. */
const agents: MessageAgents = {
  find: (number) => number === 4 || number === 5
    ? { number, description: `Agent ${number}`, run: run(number === 4 ? 'running' : 'completed') }
    : null,
  open: (number) => void done.push(`open agent ${number}`),
  stop: (number) => void done.push(`stop agent ${number}`),
}

const reply = 'Fixed in [cart.ts](src/cart.ts), committed as [Fix the total](commit:3f2a9c1) by [Cart](agent:4) after [Fonts](agent:5) ended; see [the docs](https://example.com/docs).'

/**
 * The reply as a conversation shows it, in a message whose menu is Copy
 * Message; each menu a right-click opens is recorded.
 */
async function shown(inPanel?: (url: string) => void) {
  const opened: Group[][] = []
  teardown.push(menus.installContextMenus({
    phone: () => false,
    services: { copy: () => {}, copyImage: () => {}, save: () => {}, openLink: () => {}, ...(inPanel ? { openInWorkPanel: inPanel } : {}) },
    open: (opening) => {
      opened.push(opening.menu.groups())
      return false
    },
  }))
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup() {
      provideMediaViewer()
      provideMessageFiles(() => files)
      provideMessageAgents(() => agents)
      return () => withDirectives(h('div', [h(StreamedMarkdown, { content: reply })]), [[menus.vContextMenu, () => [[{ label: 'Copy Message', run: () => {} }]]]])
    },
  }))
  app.mount(host)
  teardown.push(() => app.unmount())
  for (let turn = 0; turn < 20 && !host.querySelector('a[data-commit-link]'); turn++) {
    await nextTick()
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  /** The menu a right-click on the link whose text is `text` opens. */
  const menu = (text: string): Group[] => {
    const link = [...host.querySelectorAll('a, [data-agent-link]')].find((el) => el.textContent?.trim() === text)
    if (!link)
      throw new Error(`No link reads ${text}`)
    link.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }))
    return opened.at(-1)!
  }
  return menu
}

const labels = (groups: Group[]) => groups.map((group) => group.map((row) => row.disabled ? `(${row.label})` : row.label))
const choose = async (groups: Group[], label: string) => {
  await groups.flat().find((row) => row.label === label)!.run!()
}

test('a link to a file, a commit or an agent has that object’s menu, Open first, then its message’s; a web link adds Open in Work Panel', async () => {
  const menu = await shown(() => {})
  expect(labels(menu('cart.ts'))).toEqual([['Open', 'Show in Files'], ['Copy Path', 'Download'], ['Copy Message']])
  expect(labels(menu('Fix the total'))).toEqual([['Open'], ['Copy Commit Hash'], ['Copy Message']])
  expect(labels(menu('Cart'))).toEqual([['Open'], ['Stop Agent', 'Copy Agent ID'], ['Copy Message']])
  // A finished agent cannot be stopped.
  expect(labels(menu('Fonts'))).toEqual([['Open'], ['(Stop Agent)', 'Copy Agent ID'], ['Copy Message']])
  expect(labels(menu('the docs'))).toEqual([['Open Link', 'Open in Work Panel', 'Copy Link'], ['Copy Message']])
})

test('outside a conversation’s panel a web link has no Open in Work Panel', async () => {
  const menu = await shown()
  expect(labels(menu('the docs'))).toEqual([['Open Link', 'Copy Link'], ['Copy Message']])
})

test('each entry acts: Open as the click, Show in Files, Stop Agent on the running agent, Open in Work Panel on the link’s page', async () => {
  done.length = 0
  const menu = await shown((url) => void done.push(`panel ${url}`))
  await choose(menu('cart.ts'), 'Open')
  await choose(menu('cart.ts'), 'Show in Files')
  await choose(menu('Fix the total'), 'Open')
  await choose(menu('Cart'), 'Open')
  await choose(menu('Cart'), 'Stop Agent')
  await choose(menu('the docs'), 'Open in Work Panel')
  expect(done).toEqual([
    'open /work/src/cart.ts',
    'show in files /work/src/cart.ts',
    'open commit 3f2a9c1',
    'open agent 4',
    'stop agent 4',
    'panel https://example.com/docs',
  ])
})

test('Copy Path copies the file’s absolute path on the Host, Copy Commit Hash the full hash its abbreviation names, Copy Agent ID the agent’s number', async () => {
  const menu = await shown()
  await choose(menu('cart.ts'), 'Copy Path')
  await choose(menu('Fix the total'), 'Copy Commit Hash')
  await choose(menu('Cart'), 'Copy Agent ID')
  expect(clipboard).toEqual(['/work/src/cart.ts', FULL, '4'])
})
