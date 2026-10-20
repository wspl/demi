import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'
import type { FileContents } from '../../files/types'
import type { MessageFiles, MessageFileTarget } from '../types'

// A click on what a message shows: an image opens in the media viewer, a Host
// image as any other, and a link to a file still opens the File view. A page of
// happy-dom; a few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since the handler asks for Element.
const { createMediaViewer } = await import('../../files/media-viewer')
const { openMessageLink } = await import('../message-files')
const { renderMarkdown } = await import('../render')

const opened: string[] = []
/**
 * The Host's kept files, which a message's Host images show; never read here.
 */
const contents: FileContents = {
  url: (path) => `/raw?path=${encodeURIComponent(path)}`,
  describe: () => new Promise(() => {}),
  showDescription: () => { throw new Error('Rendering shows no file') },
}

const files: MessageFiles = {
  cwd: '/work',
  contents,
  open: (target) => void opened.push(target.path),
}

/**
 * The Host images of a message as the page shows them once their files are
 * read.
 */
const hostImages = new Map([['/work/out/chart.png', { state: 'shown' as const, src: '/raw?path=%2Fwork%2Fout%2Fchart.png&version=v1' }]])

/**
 * Renders `markdown` into the page, clicks the first `selector` match and says
 * what the viewer then shows.
 */
function click(markdown: string, selector: string) {
  document.body.innerHTML = renderMarkdown(markdown, { files, hostImages })
  const viewer = createMediaViewer(() => ({ page: { width: 1440, height: 900 }, pixelRatio: 1 }))
  opened.length = 0
  const target = document.querySelector(selector)
  if (!target)
    throw new Error(`Nothing matches ${selector}`)
  const event = new MouseEvent('click', { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'target', { value: target })
  openMessageLink(event, files, viewer, undefined)
  return { shown: viewer.shown.value, opened: [...opened], prevented: event.defaultPrevented }
}

test('a click on a Host image opens its file in the media viewer, which shows the version the message shows, not the File view', () => {
  const result = click('![The page](out/chart.png)', 'img')
  expect(result.shown).toMatchObject({ kind: 'image', file: { path: '/work/out/chart.png', contents }, name: 'The page' })
  expect(result.opened).toEqual([])
})

test('a click on a web image opens it in the media viewer too', () => {
  expect(click('![logo](https://example.com/l.png)', 'img').shown).toMatchObject({ src: 'https://example.com/l.png', name: 'logo' })
})

test('a link to a file still opens the File view, and an image inside a link follows the link', () => {
  const link = click('[chart](out/chart.png)', 'a')
  expect(link.shown).toBeNull()
  expect(link.opened).toEqual(['/work/out/chart.png'])
  const linked = click('[![chart](out/chart.png)](docs/charts.md)', 'img')
  expect(linked.shown).toBeNull()
  expect(linked.opened).toEqual(['/work/docs/charts.md'])
})

// The conversation makes its files anew each time a reply's text grows; a
// streaming reply's settled blocks render once, which holds only while their
// render reads the same object (`useMarkdownBlocks`). A few milliseconds.
test('a render reads the same files while the conversation makes them anew, and new ones when they change', async () => {
  const { effectScope, shallowRef } = await import('vue')
  const { useRenderedFiles } = await import('../message-files')
  const conversation = shallowRef<MessageFiles>({ ...files })
  const scope = effectScope()
  // Each time with a service of its own, as a subagent's messages' files are.
  const rendered = scope.run(() => useRenderedFiles(() => {
    const { open } = conversation.value
    return { ...conversation.value, ...(open ? { open: (target: MessageFileTarget) => open(target) } : {}) }
  }))!
  const first = rendered.value
  conversation.value = { ...files }
  expect(rendered.value).toBe(first)
  // A subagent's message reads its paths from another directory.
  conversation.value = { ...files, cwd: '/work/child' }
  expect(rendered.value?.cwd).toBe('/work/child')
  // A Host whose files the page cannot open shows paths as text.
  conversation.value = { cwd: '/work/child', contents }
  expect(rendered.value?.open).toBeUndefined()
  scope.stop()
})

test('a commit link opens its commit, in the repository of the folder it names before the id, resolved as a relative link’s path is', () => {
  const commits: unknown[] = []
  const withCommits: MessageFiles = { ...files, home: '/home/kim', openCommit: (target) => void commits.push(target) }
  const viewer = createMediaViewer(() => ({ page: { width: 1440, height: 900 }, pixelRatio: 1 }))
  document.body.innerHTML = renderMarkdown(
    '[One](commit:3f2a9c1) [Two](commit:src/v8@86229da) [Three](commit:/repos/shop@3f2a9c1) [Four](commit:~/shop@3f2a9c1) [Five](commit:a@b@3f2a9c1)',
    { files: withCommits },
  )
  for (const link of document.querySelectorAll('a[data-commit-link]')) {
    const event = new MouseEvent('click', { bubbles: true, cancelable: true })
    Object.defineProperty(event, 'target', { value: link })
    openMessageLink(event, withCommits, viewer, undefined)
  }
  expect(commits).toEqual([
    { commit: '3f2a9c1' },
    { commit: '86229da', root: '/work/src/v8' },
    { commit: '3f2a9c1', root: '/repos/shop' },
    { commit: '3f2a9c1', root: '/home/kim/shop' },
    // The id follows the last `@`.
    { commit: '3f2a9c1', root: '/work/a@b' },
  ])
})

test('a commit link whose folder names nothing yet, `~/` before the Host’s home is known, is its text', () => {
  const html = renderMarkdown('[Fix](commit:~/shop@3f2a9c1)', { files: { ...files, openCommit: () => {} } })
  expect(html).toBe('<p>Fix</p>\n')
})
