import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, jest, test } from 'bun:test'

// A Host image opens in the work panel's Media tab at the version of its file
// the page's kept files hold: the Host's number orders the kept files' own
// readings and its change reports, so no order of them shows an older version
// than the newest the page has heard of, a file the Host reported changed
// opens only once its new reading arrives, and each opening waits for its own
// picture.
//
// The tab is MediaView on a page of happy-dom, the medium a Host image's file;
// the Host's file is a version the test sets, its description read when the
// test lets it, and its watch the simulated one. Planted defects this
// catches: a description read before a change report shown as the file's
// version, and an opening that counts the picture of the last one as loaded.
// A few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { provideMediaViewer } = await import('../media-viewer')
const { default: MediaView } = await import('../MediaView.vue')
const { HostFiles } = await import('../file-cache')
const { keptContents } = await import('../kept-source')
const { SimulatedWatch } = await import('../memory-source')
const { WatchChannel } = await import('../watch-channel')

const PATH = '/work/chart.png'

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
  return { host, watched, contents }
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

/** The Host image's medium, which shows its file at the version the kept files hold. */
function hostImage(contents: ReturnType<typeof theHost>['contents']) {
  return { kind: 'image' as const, name: 'chart.png', file: { path: PATH, contents } }
}

test('a file the Host already reported changed never shows its old version first', async () => {
  const { host, watched, contents } = theHost()
  const message = contents.showDescription(PATH)
  await settle()
  // The change report arrived, and the file's reading again is still on its way.
  host.version = 'v2'
  host.held = true
  watched.change(PATH)
  await settle()
  const tab = mountTab()
  tab.viewer.show(hostImage(contents))
  await settle()
  expect(tab.picture()).toBeNull()
  host.release()
  await settle()
  expect(tab.picture()).toBe(`/raw?path=${PATH}&version=v2`)
  message.release()
})

test('a file changed while its tab was away shows its new version when the tab shows again, with no change report', async () => {
  const { host, contents } = theHost('unavailable')
  const tab = mountTab()
  tab.viewer.show(hostImage(contents))
  await settle()
  expect(tab.picture()).toBe(`/raw?path=${PATH}&version=v1`)
  tab.shown.value = false
  await settle()
  host.version = 'v2'
  tab.shown.value = true
  await settle()
  expect(tab.picture()).toBe(`/raw?path=${PATH}&version=v2`)
})

// Each opening waits for its own picture: opening the same image again after
// its file changed shows Loading until the new version's picture has loaded,
// and no address of the old version is asked for
// again, which the Host refuses once the file changed. Planted defects this
// catches: the first opening's loaded picture counted for the second, and the
// old version shown again before the new one's.
test('opening the same image again after its file changed shows Loading until the new picture loads', async () => {
  jest.useFakeTimers()
  try {
    const { host, watched, contents } = theHost()
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
    tab.viewer.show(hostImage(contents))
    await settle()
    load()
    await settle()
    jest.advanceTimersByTime(1000)
    await settle()
    expect([tab.picture(), loading()]).toEqual([`/raw?path=${PATH}&version=v1`, false])

    // The file changes, and the Host reports it.
    asked.clear()
    host.version = 'v2'
    watched.change(PATH)
    await settle()
    tab.viewer.show(hostImage(contents))
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
