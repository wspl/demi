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
const { default: FileBrowser } = await import('@demicodes/web-ui/files/FileBrowser.vue')
const { fileSource } = await import('./files')
const { createMemoryFileSource, dir, link, textFile } = await import('@demicodes/web-ui/files/memory-source')
type MemoryDirectory = import('@demicodes/web-ui/files/memory-source').MemoryDirectory

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
  /** While set, every request waits for it before it is answered. */
  const held: { until: Promise<void> | null } = { until: null }
  /** While set, every request is refused as the backend refuses one for a device offline. */
  const offline = { now: false }
  const notFound = (path: string) =>
    Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `No ${path}` }, { status: 404 })
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input), 'http://127.0.0.1:3271')
    const path = url.searchParams.get('path') ?? ''
    const route = url.pathname.replace('/api/conversations/c1/fs', '') || '/'
    asked.push(`${init?.method === 'HEAD' ? 'HEAD ' : ''}${route} ${path}`)
    await answering
    await held.until
    if (offline.now)
      return Response.json({ requestId: 'req_test000000000009', code: 'device_offline', message: 'runner has no connection' }, { status: 409 })
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
  return { folders, texts, asked, held, offline }
}

/** Lets the answers given land and what they started run: a few turns of the event loop, no time of its own. */
async function drain(): Promise<void> {
  for (let turn = 0; turn < 10; turn++)
    await new Promise<void>((resolve) => setImmediate(resolve))
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

/** The conversation's files on the next device, as a view reads them. */
function source() {
  devices += 1
  return fileSource(
    { directory: '/conversations/c1/fs', text: '/conversations/c1/fs/file', raw: '/conversations/c1/fs/raw' },
    { id: `d${devices}`, platform: 'linux', home: '/w' },
  )
}

/** The File view of `/w` showing `path`, which a pick in its tree replaces, as the File tab does. */
function show(path: string | null) {
  const files = source()
  const shown = ref(path)
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () => h(FileView, { source: files, root: '/w', path: shown.value, onOpen: (to: string) => { shown.value = to } }),
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

    // A link to a file shows the target's text under the link's own path; its folder is
    // listed again once, by the tree, which the File view does not repeat.
    const listings = () => backend.asked.filter((request) => request === '/ /w').length
    const before = listings()
    row('config.json').click()
    await until(shows('"fontSize": 13'), 'the target\'s text shows')
    expect(view.shown.value).toBe('/w/config.json')
    expect(backend.asked).toContain('/file /w/config.json')
    await drain()
    expect(listings()).toBe(before + 1)
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

test('a link opened by its path, as from a message, reads its folder again: a target that is back shows', async () => {
  // A Host that watches its files, so the workspace's listing stays kept as live: the link's
  // target is outside it, and its coming back changes nothing the folder's watch sees.
  const files = createMemoryFileSource({
    platform: 'linux',
    home: '/w',
    root: dir({ w: dir({ 'notes.txt': link('/srv/notes.txt', at) }), srv: dir({}) }),
  })
  const shown = ref<string | null>(null)
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({ render: () => h(FileView, { source: files, root: '/w', path: shown.value }) })
  app.mount(element)
  try {
    await until(() => rows().length === 1, 'the workspace lists')
    expect(rows()[0]![3]).toBe('Symbolic link to /srv/notes.txt, which does not exist')
    const srv = files.root.children['srv'] as MemoryDirectory
    srv.children['notes.txt'] = textFile('Back again\n', at)
    shown.value = '/w/notes.txt'
    await until(shows('Back again'), 'the file shows')
    expect(document.body.textContent).not.toContain('which does not exist')
  } finally {
    app.unmount()
  }
})

test('a link opened again shows its target once it is back, though its file was read as gone', async () => {
  // A Host that watches its files: the file's read that found nothing stays kept as live, and
  // the target's coming back, outside the workspace, changes nothing either watch sees.
  const files = createMemoryFileSource({
    platform: 'linux',
    home: '/w',
    root: dir({ w: dir({ 'notes.txt': link('/srv/notes.txt', at), 'README.md': textFile('Read me\n', at) }), srv: dir({}) }),
  })
  const shown = ref<string | null>('/w/notes.txt')
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({ render: () => h(FileView, { source: files, root: '/w', path: shown.value, onOpen: (to: string) => { shown.value = to } }) })
  app.mount(element)
  try {
    await until(shows('Symbolic link to /srv/notes.txt, which does not exist.'), 'the gone target is said')
    row('README.md').click()
    await until(shows('Read me'), 'another file shows')
    const srv = files.root.children['srv'] as MemoryDirectory
    srv.children['notes.txt'] = textFile('Back again\n', at)
    row('notes.txt').click()
    await until(shows('Back again'), 'the file shows')
  } finally {
    app.unmount()
  }
})

test('a tree that goes away while a link is read again opens nothing when the answer comes', async () => {
  const backend = host({ '/w': [linked('vendor', '../shared/vendor', { kind: 'directory' })], '/w/vendor': [folder('lib')] })
  const view = show(null)
  await until(() => rows().length === 1, 'the workspace lists')
  let answer = () => {}
  backend.held.until = new Promise<void>((resolve) => {
    answer = resolve
  })
  const before = backend.asked.length
  row('vendor').click()
  await until(() => backend.asked.length > before, 'the folder is listed again')
  view.unmount()
  answer()
  await drain()
  expect(backend.asked.slice(before)).toEqual(['/ /w'])
})

/** A file browser over `/w`, as the attachment picker (`file`) and a device's folder dialog (`directory`) show it. */
function browse(mode: 'file' | 'directory') {
  const picked: string[] = []
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () => h(FileBrowser, { mode, source: source(), initialPath: '/w', onSelect: (path: string) => picked.push(path) }),
  })
  app.mount(element)
  return { picked, unmount: () => app.unmount() }
}

function option(name: string): HTMLElement | undefined {
  return [...document.querySelectorAll<HTMLElement>('[role=option]')].find((element) => element.textContent!.trim().startsWith(name))
}

for (const mode of ['file', 'directory'] as const) {
  test(`in the ${mode === 'file' ? 'attachment picker' : 'folder dialog'}, a link listed as missing whose target is now a folder opens as one`, async () => {
    const backend = host({ '/w': [linked('data', '/mnt/data', { kind: 'missing' })] })
    const view = browse(mode)
    try {
      await until(() => option('data') !== undefined, 'the folder lists')
      backend.folders['/w'] = [linked('data', '/mnt/data', { kind: 'directory' })]
      backend.folders['/w/data'] = [file('records.csv', 1024)]
      option('data')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
      await until(() => option('records.csv') !== undefined, 'the link opens as a folder')
      expect(view.picked).toEqual([])
    } finally {
      view.unmount()
    }
  })

  test(`in the ${mode === 'file' ? 'attachment picker' : 'folder dialog'}, a link listed as a folder whose target is gone is not entered`, async () => {
    const backend = host({ '/w': [linked('data', '/mnt/data', { kind: 'directory' })], '/w/data': [file('records.csv', 1024)] })
    const view = browse(mode)
    try {
      await until(() => option('data') !== undefined, 'the folder lists')
      backend.folders['/w'] = [linked('data', '/mnt/data', { kind: 'missing' })]
      delete backend.folders['/w/data']
      option('data')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
      // Neither takes the link: the attachment picker says why, the folder dialog's row is now a file's.
      // The row turns a file's, with its size, as the listing read again lands; the picker answers as that read ends.
      await until(() => option('data')?.textContent!.includes('9 B') === true, 'the row says what the link is now')
      await drain()
      if (mode === 'file')
        await until(shows('Symbolic link to /mnt/data, which does not exist.'), 'the picker says why')
      expect(view.picked).toEqual([])
      expect(backend.asked).not.toContain('/ /w/data')
    } finally {
      view.unmount()
    }
  })
}

test('the attachment picker attaches no link whose target cannot be read, and says why', async () => {
  const backend = host({ '/w': [linked('secret', '/root/notes', { kind: 'file' })] }, { '/w/secret': 'hidden' })
  const view = browse('file')
  try {
    await until(() => option('secret') !== undefined, 'the folder lists')
    // The folder on the way to the target closes after the listing: the row still shows a file.
    backend.folders['/w'] = [linked('secret', '/root/notes', { kind: 'unreadable', reason: 'Permission denied' })]
    option('secret')!.click()
    const open = () => [...document.querySelectorAll<HTMLElement>('[role=button]')].find((button) => button.textContent!.trim() === 'Open')!
    await until(() => open().getAttribute('aria-disabled') !== 'true', 'Open is offered for the selected file')
    open().click()
    await until(shows('Symbolic link to /root/notes, which cannot be read: permission denied.'), 'the picker says why')
    expect(view.picked).toEqual([])
  } finally {
    view.unmount()
  }
})

test('a link whose folder cannot be listed again is neither chosen nor opened, and the picker says why', async () => {
  const backend = host({ '/w': [linked('notes.txt', '/srv/notes.txt', { kind: 'file' })] }, { '/w/notes.txt': 'Notes\n' })
  const view = browse('file')
  try {
    await until(() => option('notes.txt') !== undefined, 'the folder lists')
    backend.offline.now = true
    option('notes.txt')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await until(shows('The device is offline.'), 'the picker says why')
    expect(view.picked).toEqual([])
  } finally {
    view.unmount()
  }
})

test('a link in the tree whose folder cannot be listed again does not open', async () => {
  const backend = host({ '/w': [linked('notes.txt', '/srv/notes.txt', { kind: 'file' })] }, { '/w/notes.txt': 'Notes\n' })
  const view = show(null)
  try {
    await until(() => rows().length === 1, 'the workspace lists')
    backend.offline.now = true
    const before = backend.asked.length
    row('notes.txt').click()
    await until(() => backend.asked.length > before, 'the folder is listed again')
    await drain()
    expect(view.shown.value).toBe(null)
    expect(backend.asked.slice(before)).toEqual(['/ /w'])
  } finally {
    view.unmount()
  }
})

test('the link shown, opened again from the tree, reads its file again: a target that is back shows', async () => {
  const backend = host({ '/w': [linked('notes.txt', '/srv/notes.txt', { kind: 'missing' })] })
  const view = show('/w/notes.txt')
  try {
    await until(shows('Symbolic link to /srv/notes.txt, which does not exist.'), 'the gone target is said')
    backend.folders['/w'] = [linked('notes.txt', '/srv/notes.txt', { kind: 'file' })]
    backend.texts['/w/notes.txt'] = 'Back again\n'
    const listings = () => backend.asked.filter((request) => request === '/ /w').length
    const before = listings()
    row('notes.txt').click()
    await until(shows('Back again'), 'the file shows')
    expect(view.shown.value).toBe('/w/notes.txt')
    // The tree listed the folder again; the File view only read the file.
    await drain()
    expect(listings()).toBe(before + 1)
  } finally {
    view.unmount()
  }
})
