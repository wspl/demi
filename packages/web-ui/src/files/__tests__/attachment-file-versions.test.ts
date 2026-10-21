import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, jest, test } from 'bun:test'

// An agent's picture attachment opens its Host file in the Media tab, and the
// answer of whether it does carries the file's description, read at the
// Host's number, and whether that version holds the copy's bytes. The page's
// kept files own the file's description: the answer enters them as one more
// reading, so whatever order the answer, the kept files' own reads and the
// Host's change reports come in, the newest reading decides what shows, and
// the copy shows only for the version it was answered for.
//
// The tab is MediaView on a page of happy-dom, the medium the one a click on
// the attachment opens (`attachmentMedium`); the Host's file is a version the
// test sets, its description read when the test lets it, and its watch the
// simulated one. Planted defects this catches: the answer used in place of
// the kept files' read, which shows an outdated version, or the copy of one,
// in each of the three orders below. A few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { provideMediaViewer } = await import('../media-viewer')
const { default: MediaView } = await import('../MediaView.vue')
const { HostFiles } = await import('../file-cache')
const { keptContents } = await import('../kept-source')
const { SimulatedWatch } = await import('../memory-source')
const { WatchChannel } = await import('../watch-channel')
const { attachmentMedium } = await import('../../markdown/render')

const PATH = '/work/chart.png'
const COPY = '/blobs/chart'

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** Lets the page's answers land and the tab render them; the event loop turns even under a paused clock. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 6; turn++) {
    await new Promise((resolve) => setImmediate(resolve))
    await nextTick()
  }
}

/**
 * The Host: the file's version now, its watch, and its kept files, whose
 * reads of the description wait while `held` holds them.
 */
function theHost(mode: 'live' | 'unavailable' = 'live') {
  const kept = new HostFiles()
  const watched = new SimulatedWatch(mode)
  const host = { version: 'v1', held: false, release: () => {} }
  const contents = keptContents({
    url: (path, options = {}) => `/raw?path=${path}&version=${options.version ?? ''}`,
    async describe() {
      const readAt = watched.readAt()
      const version = host.version
      if (host.held)
        await new Promise<void>((resolve) => (host.release = resolve))
      return { value: { size: 9, modifiedAt: null, version }, readAt }
    },
  }, { files: kept, watch: new WatchChannel(kept, watched.open) })
  /** What the Host answers of the file now, as the attachment's question carries it. */
  const answer = (sameAsCopy: boolean) => ({
    path: PATH,
    described: { value: { size: 9, modifiedAt: null, version: host.version }, readAt: watched.readAt() },
    sameAsCopy,
  })
  return { host, watched, contents, answer }
}

/** The Media tab, shown while `shown` holds. */
function mountTab() {
  const root = document.createElement('div')
  document.body.append(root)
  const shown = ref(true)
  let viewer: ReturnType<typeof provideMediaViewer> | null = null
  const app = createApp(defineComponent({
    setup() {
      viewer = provideMediaViewer({ reveal: () => true })
      return () => h(MediaView, { shown: shown.value })
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  const picture = () => root.querySelector('[data-media-view] img')?.getAttribute('src') ?? null
  return { viewer: viewer!, shown, picture }
}

/** The attachment's medium, whose question answers what `answer` gives when asked. */
function attachment(contents: ReturnType<typeof theHost>['contents'], answer: () => ReturnType<ReturnType<typeof theHost>['answer']>) {
  return attachmentMedium('a1', {
    name: 'chart.png', mediaType: 'image/png', sha256: 'c'.repeat(64), url: COPY, size: 9, width: 4, height: 3,
    source: { host: 'dev-1', path: PATH },
  }, { cwd: '/work', contents, attachmentOrigin: async () => answer() })!
}

test('an answer newer than the kept description shows its version, though the kept one counts as current', async () => {
  const { host, watched, contents, answer } = theHost()
  // A Host image of a message shows the file, so the kept files hold v1 as current.
  const message = contents.showDescription(PATH)
  await settle()
  expect(message.entry.value?.version).toBe('v1')
  // The file changes, and the Host answers the attachment's question before its
  // change report reaches the page.
  host.version = 'v2'
  watched.change('/elsewhere/other.txt')
  const tab = mountTab()
  tab.viewer.show(attachment(contents, () => answer(false)))
  await settle()
  expect(tab.picture()).toBe(`/raw?path=${PATH}&version=v2`)
  message.release()
})

test('an answer the Host already reported changed never shows its copy first', async () => {
  const { host, watched, contents, answer } = theHost()
  const message = contents.showDescription(PATH)
  await settle()
  // Answered while the file still held the copy's bytes, then it changed, and
  // the change report arrived before the answer.
  // Its reading again is still on its way.
  const stale = answer(true)
  host.version = 'v2'
  host.held = true
  watched.change(PATH)
  await settle()
  const tab = mountTab()
  tab.viewer.show(attachment(contents, () => stale))
  await settle()
  expect(tab.picture()).toBeNull()
  host.release()
  await settle()
  expect(tab.picture()).toBe(`/raw?path=${PATH}&version=v2`)
  message.release()
})

test('a file changed while its tab was away shows its new version when the tab shows again, with no change report', async () => {
  const { host, contents, answer } = theHost('unavailable')
  const tab = mountTab()
  tab.viewer.show(attachment(contents, () => answer(true)))
  await settle()
  expect(tab.picture()).toBe(COPY)
  tab.shown.value = false
  await settle()
  host.version = 'v2'
  tab.shown.value = true
  await settle()
  expect(tab.picture()).toBe(`/raw?path=${PATH}&version=v2`)
})

// Each opening waits for its own picture: opening the same attachment again
// after its file changed shows Loading until the new version's picture has
// loaded, not only until the question is answered, and no address of the
// version the first answer described is ever asked for, which the Host
// refuses once the file changed. Planted defects this catches: the first
// opening's loaded picture counted for the second, and the first answer's
// description shown again before the second's.
test('opening the same attachment again after its file changed shows Loading until the new picture loads', async () => {
  jest.useFakeTimers()
  try {
    const { host, watched, contents, answer } = theHost()
    const tab = mountTab()
    const asked = new Set<string>()
    const observer = new MutationObserver(() => {
      for (const image of document.querySelectorAll('img'))
        asked.add(image.getAttribute('src') ?? '')
    })
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['src'] })
    const loading = () => document.querySelector('[data-media-view] [aria-busy="true"]') !== null
    const load = () => {
      const image = document.querySelector<HTMLImageElement>('[data-media-view] img')!
      Object.defineProperty(image, 'naturalWidth', { value: 4, configurable: true })
      Object.defineProperty(image, 'naturalHeight', { value: 3, configurable: true })
      image.dispatchEvent(new Event('load'))
    }
    tab.viewer.show(attachment(contents, () => answer(true)))
    await settle()
    load()
    await settle()
    jest.advanceTimersByTime(1000)
    await settle()
    expect([tab.picture(), loading()]).toEqual([COPY, false])

    // The file changes; the Host answers the question at a later number,
    // before its change report reaches the page.
    host.version = 'v2'
    watched.change('/elsewhere/other.txt')
    tab.viewer.show(attachment(contents, () => answer(false)))
    await settle()
    jest.advanceTimersByTime(1000)
    await settle()
    expect([tab.picture(), loading()]).toEqual([`/raw?path=${PATH}&version=v2`, true])
    load()
    await settle()
    expect(loading()).toBe(false)
    observer.disconnect()
    expect([...asked].filter((src) => src.includes('version=v1'))).toEqual([])
  } finally {
    jest.useRealTimers()
  }
})
