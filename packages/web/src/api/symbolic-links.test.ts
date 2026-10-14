import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A conversation's File view shows symbolic links as their targets
// (`file-previews.md` § Symbolic links), on a page of happy-dom whose
// backend is a stubbed fetch answering the file routes as the backend
// does, from a Host the test changes between steps. The workspace `/w`
// holds `vendor`, a link to a folder, `config.json`, a link to a file,
// `old.log`, a link to a file that is gone, and `secret`, a link behind a
// folder closed to the user. Cost: no I/O; tens of milliseconds a test.

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
type LeadsTo = { kind: 'file' | 'directory' | 'missing' } | { kind: 'unreadable'; reason: string }
const folder = (name: string) => ({ name, isDirectory: true, size: 96, modifiedAt: at })
const file = (name: string, size: number) => ({ name, isDirectory: false, size, modifiedAt: at })
const linked = (name: string, target: string, leadsTo: LeadsTo) =>
  ({ name, isDirectory: false, size: target.length, modifiedAt: at, link: { target, leadsTo } })

/**
 * A Host as the backend answers of it: each folder's listing and each
 * file's text by path, which a test changes as something on the Host would,
 * and every request the page made, as `<route> <path>`.
 */
function host(folders: Record<string, object[]>, texts: Record<string, string> = {}, answering: Promise<void> = Promise.resolve()) {
  const asked: string[] = []
  const notFound = (path: string) =>
    Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `No ${path}` }, { status: 404 })
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input), 'http://127.0.0.1:3271')
    const path = url.searchParams.get('path') ?? ''
    const route = url.pathname.replace('/api/conversations/c1/fs', '') || '/'
    asked.push(`${init?.method === 'HEAD' ? 'HEAD ' : ''}${route} ${path}`)
    await answering
    if (route === '/') {
      const entries = folders[path]
      return entries ? Response.json({ path, home: '/w', entries }) : notFound(path)
    }
    if (route === '/file') {
      const text = texts[path]
      return text === undefined ? notFound(path) : Response.json({ path, text }, { headers: { etag: `W/"${text.length}-1"` } })
    }
    if (route === '/raw' && init?.method === 'HEAD') {
      const text = texts[path]
      return text === undefined
        ? new Response(null, { status: 404 })
        : new Response(null, { headers: { 'content-length': String(text.length), 'last-modified': 'Sat, 12 Sep 2026 09:30:00 GMT', etag: `W/"${text.length}-1"` } })
    }
    throw new Error(`Unexpected request: ${route} ${path}`)
  }, { preconnect: realFetch.preconnect })
  return { folders, texts, asked }
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

const shows = (text: string) => () => document.body.textContent!.includes(text)

/** The tree's rows, each its name, depth, whether it folds, and its link mark's words. */
function rows(): Array<[string, string, string | null, string | null]> {
  return [...document.querySelectorAll<HTMLElement>('[role=treeitem]')].map((row) => [
    row.textContent!.trim(),
    row.style.paddingLeft,
    row.getAttribute('aria-expanded'),
    row.querySelector('[role=img][aria-label^="Symbolic link"]')?.getAttribute('aria-label') ?? null,
  ])
}

function row(name: string, depth = 0): HTMLElement {
  const found = [...document.querySelectorAll<HTMLElement>('[role=treeitem]')]
    .find((element) => element.textContent!.trim() === name && element.style.paddingLeft === `${4 + depth * 12}px`)
  if (!found)
    throw new Error(`No row ${name} at depth ${depth}: ${JSON.stringify(rows())}`)
  return found
}

/** Devices shown so far: the page keeps what it read of a device's files for its lifetime, so each test has its own. */
let devices = 0

/** The File view of `/w` showing `path`, which a pick in its tree replaces, as the File tab does. */
function show(path: string | null) {
  devices += 1
  const source = fileSource(
    { directory: '/conversations/c1/fs', text: '/conversations/c1/fs/file', raw: '/conversations/c1/fs/raw' },
    { id: `d${devices}`, platform: 'linux', home: '/w' },
  )
  const shown = ref(path)
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () => h(FileView, { source, root: '/w', path: shown.value, onOpen: (to: string) => { shown.value = to } }),
  })
  app.mount(element)
  return { shown, unmount: () => app.unmount() }
}

const workspace = () => [
  folder('src'),
  linked('vendor', '../shared/vendor', { kind: 'directory' }),
  linked('config.json', '/home/ana/.config/app.json', { kind: 'file' }),
  linked('old.log', '/gone/old.log', { kind: 'missing' }),
  linked('secret', '/root/notes', { kind: 'unreadable', reason: 'Permission denied' }),
]

test('links show as their targets, unfold at any depth, open their target, and a broken one says why, whatever its read answered', async () => {
  const backend = host({
    '/w': workspace(),
    '/w/vendor': [folder('lib'), linked('project', '/w', { kind: 'directory' })],
    '/w/vendor/project': workspace(),
  }, { '/w/config.json': '{\n  "theme": "dark",\n  "fontSize": 13\n}\n' })
  const view = show('/w/old.log')
  try {
    // The sentence is the listing's, whatever the read of the file answered.
    await until(shows('Symbolic link to /gone/old.log, which does not exist.'), 'the gone target is said')
    expect(document.body.textContent).not.toContain('This file does not exist.')
    expect(rows()).toEqual([
      ['src', '4px', 'false', null],
      ['vendor', '4px', 'false', 'Symbolic link to ../shared/vendor'],
      ['config.json', '4px', null, 'Symbolic link to /home/ana/.config/app.json'],
      ['old.log', '4px', null, 'Symbolic link to /gone/old.log, which does not exist'],
      ['secret', '4px', null, 'Symbolic link to /root/notes, which cannot be read: permission denied'],
    ])

    row('secret').click()
    await until(shows('Symbolic link to /root/notes, which cannot be read: permission denied.'), 'the unreadable target is said')

    // A link to a folder unfolds into the target's entries, listed by the link's own path, a link back up again.
    row('vendor').click()
    await until(() => rows().some(([name]) => name === 'project'), 'vendor unfolds')
    row('project', 1).click()
    await until(() => rows().some(([name, depth]) => name === 'config.json' && depth === '28px'), 'project unfolds')
    expect(rows().slice(1, 5)).toEqual([
      ['vendor', '4px', 'true', 'Symbolic link to ../shared/vendor'],
      ['lib', '16px', 'false', null],
      ['project', '16px', 'true', 'Symbolic link to /w'],
      ['src', '28px', 'false', null],
    ])
    expect(backend.asked).toContain('/ /w/vendor/project')

    // A link to a file shows the target's text under the link's own path.
    row('config.json').click()
    await until(shows('"fontSize": 13'), 'the target\'s text shows')
    expect(view.shown.value).toBe('/w/config.json')
    expect(backend.asked).toContain('/file /w/config.json')
  } finally {
    view.unmount()
  }
})

test('expanding or opening a link reads its target again: gone now says so, there now expands', async () => {
  const backend = host({
    '/w': [linked('data', '/mnt/data', { kind: 'missing' }), linked('vendor', '../shared/vendor', { kind: 'directory' })],
    '/w/vendor': [folder('lib')],
  })
  const view = show(null)
  try {
    await until(() => rows().length === 2, 'the workspace lists')
    // The target appears on the Host, and the vendor's goes: the rows say what the listing said.
    backend.folders['/w'] = [linked('data', '/mnt/data', { kind: 'directory' }), linked('vendor', '../shared/vendor', { kind: 'missing' })]
    backend.folders['/w/data'] = [file('records.csv', 1024)]
    delete backend.folders['/w/vendor']
    expect(rows()).toEqual([
      ['vendor', '4px', 'false', 'Symbolic link to ../shared/vendor'],
      ['data', '4px', null, 'Symbolic link to /mnt/data, which does not exist'],
    ])

    row('data').click()
    await until(() => rows().some(([name]) => name === 'records.csv'), 'the link now to a folder expands')
    expect(row('data').getAttribute('aria-expanded')).toBe('true')
    expect(view.shown.value).toBe(null)

    row('vendor').click()
    await until(shows('Symbolic link to ../shared/vendor, which does not exist.'), 'the link whose folder went says so')
    expect(view.shown.value).toBe('/w/vendor')
    expect(rows().find(([name]) => name === 'vendor')).toEqual(['vendor', '4px', null, 'Symbolic link to ../shared/vendor, which does not exist'])
  } finally {
    view.unmount()
  }
})

test('a link to an image that is gone says so, and Retry shows the image once its target is back', async () => {
  const backend = host({ '/w': [linked('logo.png', '/srv/logo.png', { kind: 'missing' })] })
  const view = show('/w/logo.png')
  try {
    await until(shows('Symbolic link to /srv/logo.png, which does not exist.'), 'the gone image is said')

    backend.folders['/w'] = [linked('logo.png', '/srv/logo.png', { kind: 'file' })]
    backend.texts['/w/logo.png'] = 'PNG'
    const retry = [...document.querySelectorAll<HTMLElement>('button, [role=button]')].find((button) => button.textContent!.trim() === 'Retry')!
    retry.click()
    await until(() => document.querySelector('img[src*="logo.png"]') !== null, 'the image shows')
    expect(backend.asked).toContain('HEAD /raw /w/logo.png')
  } finally {
    view.unmount()
  }
})

test('a file is asked for at once with its folder\'s listing, not after it', async () => {
  // Nothing is answered until both are asked: a read that waited for the listing would never be asked.
  let answer = () => {}
  const answering = new Promise<void>((resolve) => {
    answer = resolve
  })
  const backend = host({ '/': [folder('w')], '/w': [folder('docs')], '/w/docs': [file('notes.md', 6)] }, { '/w/docs/notes.md': 'Notes\n' }, answering)
  const view = show('/w/docs/notes.md')
  try {
    const both = ['/ /w/docs', '/file /w/docs/notes.md']
    await until(() => both.every((request) => backend.asked.includes(request)), 'the listing and the file are both asked for')
    answer()
    await until(shows('Notes'), 'the file shows')
  } finally {
    answer()
    view.unmount()
  }
})
