import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A conversation's File view shows symbolic links as their targets
// (`file-previews.md` § Symbolic links): the workspace `/w` holds `vendor`,
// a link to a folder, `config.json`, a link to a file, and `old.log`, a link
// to a file that is gone, as the backend's listing names them. The tree
// shows `vendor` as a folder that unfolds into the target's entries, a link
// back to `/w` among them unfolding again a level deeper; each link wears
// its mark naming the target; a file link opens under its own path; and the
// gone one, opened, says so where the file would be. The page is happy-dom,
// the backend a stubbed fetch answering the routes as it does. Cost: no
// I/O; tens of milliseconds.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
// After the page, since the components ask for its elements.
await import('../__tests__/vue-components')
const { createApp, h, ref } = await import('vue')
const { default: FileView } = await import('@demicodes/web-ui/files/FileView.vue')
const { fileSource } = await import('./files')

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
  document.body.innerHTML = ''
})

const at = '2026-09-12T09:30:00.000Z'
const folder = (name: string) => ({ name, isDirectory: true, size: 96, modifiedAt: at })
const linked = (name: string, target: string, targetKind: string) =>
  ({ name, isDirectory: false, size: target.length, modifiedAt: at, link: { target, targetKind } })
const workspace = [
  folder('src'),
  linked('vendor', '../shared/vendor', 'directory'),
  linked('config.json', '/home/ana/.config/app.json', 'file'),
  linked('old.log', '/gone/old.log', 'missing'),
]
const vendor = [folder('lib'), linked('project', '/w', 'directory')]

/** Answers the listing and text routes from the fixture, noting each path the page asked for. */
function stubBackend(): string[] {
  const asked: string[] = []
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://127.0.0.1:3271')
    const path = url.searchParams.get('path') ?? ''
    asked.push(`${url.pathname} ${path}`)
    if (url.pathname === '/api/conversations/c1/fs') {
      const entries = path === '/w' || path === '/w/vendor/project' ? workspace : path === '/w/vendor' ? vendor : null
      return entries
        ? Response.json({ path, home: '/w', entries })
        : Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `No ${path}` }, { status: 404 })
    }
    if (url.pathname === '/api/conversations/c1/fs/file' && path === '/w/old.log')
      return Response.json({ requestId: 'req_test000000000002', code: 'not_found', message: 'No such file' }, { status: 404 })
    throw new Error(`Unexpected request: ${url.pathname} ${path}`)
  }, { preconnect: realFetch.preconnect })
  return asked
}

/** Resolves once `done` holds, checked at each change of the page. */
function until(done: () => boolean, what: string): Promise<void> {
  return new Promise((resolve, reject) => {
    if (done())
      return resolve()
    const timer = setTimeout(() => {
      observer.disconnect()
      reject(new Error(`${what} did not happen: ${document.body.textContent}`))
    }, 2_000)
    const observer = new MutationObserver(() => {
      if (done()) {
        observer.disconnect()
        clearTimeout(timer)
        resolve()
      }
    })
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true })
  })
}

/** The tree's rows, each its name, depth, whether it folds, and its link mark's words. */
function rows(): Array<[string, string, string | null, string | null]> {
  return [...document.querySelectorAll<HTMLElement>('[role=treeitem]')].map((row) => [
    row.textContent!.trim(),
    row.style.paddingLeft,
    row.getAttribute('aria-expanded'),
    row.querySelector('[role=img][aria-label^="Symbolic link"]')?.getAttribute('aria-label') ?? null,
  ])
}

function row(name: string, depth: number): HTMLElement {
  const found = [...document.querySelectorAll<HTMLElement>('[role=treeitem]')]
    .find((element) => element.textContent!.trim() === name && element.style.paddingLeft === `${4 + depth * 12}px`)
  if (!found)
    throw new Error(`No row ${name} at depth ${depth}: ${JSON.stringify(rows())}`)
  return found
}

test('links show as their targets in the tree, unfold at any depth, and a gone target is said where the file would be', async () => {
  const asked = stubBackend()
  const source = fileSource(
    { directory: '/conversations/c1/fs', text: '/conversations/c1/fs/file' },
    { id: 'd1', platform: 'linux', home: '/w' },
  )
  const path = ref('/w/old.log')
  const opened: string[] = []
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () => h(FileView, { source, root: '/w', path: path.value, onOpen: (to: string) => opened.push(to) }),
  })
  app.mount(element)
  try {
    await until(() => document.body.textContent!.includes('which does not exist.'), 'the gone target is said')
    expect(document.body.textContent).toContain('Symbolic link to /gone/old.log, which does not exist.')
    expect(rows()).toEqual([
      ['src', '4px', 'false', null],
      ['vendor', '4px', 'false', 'Symbolic link to ../shared/vendor'],
      ['config.json', '4px', null, 'Symbolic link to /home/ana/.config/app.json'],
      ['old.log', '4px', null, 'Symbolic link to /gone/old.log, which does not exist'],
    ])

    // The link to a folder unfolds into the target's entries, listed by the link's own path.
    row('vendor', 0).click()
    await until(() => rows().some(([name]) => name === 'project'), 'vendor unfolds')
    row('project', 1).click()
    await until(() => rows().some(([name, depth]) => name === 'config.json' && depth === '28px'), 'project unfolds')
    expect(rows().slice(1, 5)).toEqual([
      ['vendor', '4px', 'true', 'Symbolic link to ../shared/vendor'],
      ['lib', '16px', 'false', null],
      ['project', '16px', 'true', 'Symbolic link to /w'],
      ['src', '28px', 'false', null],
    ])
    expect(asked.filter((request) => request.startsWith('/api/conversations/c1/fs '))).toEqual([
      '/api/conversations/c1/fs /w',
      '/api/conversations/c1/fs /w/vendor',
      '/api/conversations/c1/fs /w/vendor/project',
    ])

    // A link to a file opens under its own path, which the Host reads through the link.
    row('config.json', 0).click()
    expect(opened).toEqual(['/w/config.json'])
  } finally {
    app.unmount()
  }
})
