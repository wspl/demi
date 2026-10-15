import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import type { EffectScope } from 'vue'

// An image in a Markdown document shows its file as it is now
// (`file-previews.md` § Files named in messages): README.md names
// `shots/login.png`, the file is overwritten while the document is open, and
// the image shows the new picture in place, learned from the Host's watch,
// without the document rendering again. The web browser keeps the picture
// it decoded for an address for as long as the page lives, so the new
// version must be a new address. Played against a simulated Host's watch on
// a page of happy-dom, which cannot render the Markdown itself (its
// sanitizer drops the document's structure): the rendering is the one the
// document renderer gives for such an image, a path and no address. Cost: no I/O, fake timers; a few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since Vue and the components ask for its document and classes.
const { createApp, effectScope } = await import('vue')
const { HostFiles } = await import('../file-cache')
const { keptContents } = await import('../kept-source')
const { SimulatedWatch } = await import('../memory-source')
const { WatchChannel } = await import('../watch-channel')
const { useDocumentImages } = await import('../document-images')

beforeEach(() => {
  jest.useFakeTimers()
})

let open: EffectScope | null = null
afterEach(() => {
  open?.stop()
  open = null
  document.body.innerHTML = ''
  jest.useRealTimers()
})

/** Lets the Host's answers and its watch's messages land: they land in microtasks. */
async function settle(): Promise<void> {
  for (let tick = 0; tick < 20; tick++)
    await Promise.resolve()
}

const PATH = '/w/docs/shots/login.png'

function host() {
  const watched = new SimulatedWatch()
  const kept = new HostFiles()
  const watch = new WatchChannel(kept, watched.open)
  let version = '"v1"'
  const contents = keptContents({
    url: (path, options = {}) => `/raw?path=${path}${options.version ? `&version=${options.version}` : ''}`,
    async describe(path) {
      if (path !== PATH)
        throw new Error('The file is not there')
      return { value: { size: 4096, modifiedAt: null, version }, readAt: watched.readAt() }
    },
  }, { files: kept, watch })
  /** Something on the Host overwrites the file, as a new screenshot does. */
  const overwrite = (next: string) => {
    version = next
    watched.change(PATH)
  }
  return { contents, overwrite }
}

/** The page's app, whose context the document's component runs in. */
const app = createApp({})

/**
 * The document's rendering on the page, as the File view shows it, whose
 * images the document gives their addresses.
 */
function show(html: string, hostFiles: string[], contents: ReturnType<typeof host>['contents']) {
  const element = document.createElement('div')
  element.innerHTML = html
  document.body.append(element)
  open = effectScope()
  const images = app.runWithContext(() => open!.run(() => useDocumentImages({
    root: () => element,
    html: () => html,
    hostFiles: () => hostFiles,
    contents: () => contents,
  }))!)
  element.addEventListener('error', images.onError, true)
  return { images: () => [...element.querySelectorAll('img')], text: () => element.textContent?.trim(), reload: images.reload }
}

const LOGIN = `<img data-host-file="${PATH}" alt="The login page">`

test('a document’s Host image shows the new version when its file is overwritten, in place', async () => {
  const { contents, overwrite } = host()
  const document_ = show(`<h1>Login</h1><p>${LOGIN}</p>`, [PATH], contents)
  await settle()
  const [image] = document_.images()
  expect(image?.getAttribute('src')).toBe(`/raw?path=${PATH}&version="v1"`)
  expect(image?.alt).toBe('The login page')

  overwrite('"v2"')
  await settle()
  // The same element: the document was not rendered again.
  expect(document_.images()[0]).toBe(image)
  expect(image?.getAttribute('src')).toBe(`/raw?path=${PATH}&version="v2"`)
})

test('a document’s Host image has no address until its file is read, and a web image has its own', async () => {
  const { contents } = host()
  const document_ = show(`<p>${LOGIN} <img src="https://example.com/badge.svg" alt="web"></p>`, [PATH], contents)
  const [local, web] = document_.images()
  expect(local?.hasAttribute('src')).toBe(false)
  expect(web?.getAttribute('src')).toBe('https://example.com/badge.svg')
  await settle()
  expect(local?.getAttribute('src')).toBe(`/raw?path=${PATH}&version="v1"`)
})

test('an image that failed to load tries the new version of its file', async () => {
  const { contents, overwrite } = host()
  const document_ = show(`<p>${LOGIN}</p>`, [PATH], contents)
  await settle()
  const [image] = document_.images()
  image?.dispatchEvent(new Event('error'))
  expect(document_.images()).toHaveLength(0)
  expect(document_.text()).toBe('The login page')

  overwrite('"v2"')
  await settle()
  expect(document_.images()).toEqual([image!])
  expect(image?.getAttribute('src')).toBe(`/raw?path=${PATH}&version="v2"`)
})

test('a picture’s Host srcset follows its file at its own address, and a candidate whose file is not there drops out', async () => {
  const { contents, overwrite } = host()
  const candidates = JSON.stringify([
    { path: PATH, descriptor: '1x' },
    { path: '/w/docs/shots/gone.png', descriptor: '2x' },
    { url: 'https://example.com/login@3x.png', descriptor: '3x' },
  ])
  const document_ = show(`<picture><source data-host-srcset='${candidates}'>${LOGIN}</picture>`, [PATH, '/w/docs/shots/gone.png'], contents)
  const source = () => document.querySelector('source')
  expect(source()?.hasAttribute('srcset')).toBe(false)
  await settle()
  expect(source()?.getAttribute('srcset')).toBe(`/raw?path=${PATH}&version="v1" 1x, https://example.com/login@3x.png 3x`)

  overwrite('"v2"')
  await settle()
  expect(source()?.getAttribute('srcset')).toBe(`/raw?path=${PATH}&version="v2" 1x, https://example.com/login@3x.png 3x`)
  expect(document_.images()[0]?.getAttribute('src')).toBe(`/raw?path=${PATH}&version="v2"`)
})

test('an image whose file is not there shows its alt text in a box', async () => {
  const { contents } = host()
  const document_ = show('<p><img data-host-file="/w/docs/shots/gone.png" alt="The gone page"></p>', ['/w/docs/shots/gone.png'], contents)
  await settle()
  expect(document_.images()).toHaveLength(0)
  expect(document_.text()).toBe('The gone page')
})
