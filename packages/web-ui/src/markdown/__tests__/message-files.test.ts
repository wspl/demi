import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'
import type { MessageFiles } from '../types'

// A click on what a message shows (`file-previews.md` § Files named in
// messages): an image opens in the media viewer, a Host image as any other,
// and a link to a file still opens the File view. A page of happy-dom; a few
// milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since the handler asks for Element.
const { createMediaViewer } = await import('../../files/media-viewer')
const { openMessageLink } = await import('../message-files')
const { renderMarkdown } = await import('../render')

const opened: string[] = []
const files: MessageFiles = {
  cwd: '/work',
  imageUrl: (path) => `/raw?path=${encodeURIComponent(path)}`,
  open: (target) => void opened.push(target.path),
}

/** Renders `markdown` into the page, clicks the first `selector` match and says what the viewer then shows. */
function click(markdown: string, selector: string) {
  document.body.innerHTML = renderMarkdown(markdown, { files })
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

test('a click on a Host image opens it in the media viewer, not the File view', () => {
  const result = click('![The page](out/chart.png)', 'img')
  expect(result.shown).toMatchObject({ kind: 'image', src: `/raw?path=${encodeURIComponent('/work/out/chart.png')}`, name: 'The page' })
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
