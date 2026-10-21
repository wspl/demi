import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A video in the work panel's Media tab plays only while the tab shows, and a
// tab hidden while its video's source is still asked for shows the video once
// it is shown again.
//
// The tab is MediaView on a page of happy-dom, given the page's viewer. An
// attachment's question of whether it opens its file is a promise the test
// settles, the Host's file a description the test answers. Happy-dom loads no
// media, so the test plays a web browser's part where a test needs it: a
// player given no address fails. A few milliseconds besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { provideMediaViewer } = await import('../media-viewer')
const { default: MediaView } = await import('../MediaView.vue')
const { HostFiles } = await import('../file-cache')
const { keptContents } = await import('../kept-source')
const { SimulatedWatch } = await import('../memory-source')
const { WatchChannel } = await import('../watch-channel')

type MediaViewerState = ReturnType<typeof provideMediaViewer>
type MediumFile = { path: string; contents: ReturnType<typeof keptContents> }

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** Lets the page's answers land and the tab render them. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 4; turn++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
    await nextTick()
  }
}

/**
 * The Host's files, which hold `/work/clip.webm`; its description arrives
 * when the test calls `describe`, as over a slow connection.
 */
function hostContents() {
  const kept = new HostFiles()
  const watched = new SimulatedWatch()
  let describe: () => void = () => {}
  const described = new Promise<void>((resolve) => {
    describe = resolve
  })
  const contents = keptContents({
    url: (path) => `/raw?path=${path}`,
    describe: async () => {
      await described
      return { value: { size: 9, modifiedAt: null, version: null }, readAt: watched.readAt() }
    },
  }, { files: kept, watch: new WatchChannel(kept, watched.open) })
  return { contents, describe: () => describe() }
}

/** The Media tab, shown while `shown` holds, and the page's viewer. */
function mountTab(): { root: HTMLElement; viewer: MediaViewerState; shown: { value: boolean } } {
  const root = document.createElement('div')
  document.body.append(root)
  const shown = ref(true)
  let viewer: MediaViewerState | null = null
  const app = createApp(defineComponent({
    setup() {
      viewer = provideMediaViewer({ reveal: () => true })
      return () => h(MediaView, { shown: shown.value })
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  return { root, viewer: viewer!, shown }
}

/** The agent's video, whose question of whether it opens its file the test answers with `answer`. */
function attachedVideo() {
  let answer: (file: MediumFile | null) => void = () => {}
  const asked = new Promise<MediumFile | null>((resolve) => {
    answer = resolve
  })
  const medium = { kind: 'video' as const, name: 'clip.webm', src: '/blob/clip', origin: () => asked }
  return { medium, answer: (file: MediumFile | null) => answer(file) }
}

/** What a web browser does to a player given no address: it fails. */
function failPlayersWithoutAddress(root: HTMLElement): void {
  for (const video of root.querySelectorAll('video')) {
    if (!video.getAttribute('src'))
      video.dispatchEvent(new Event('error'))
  }
}

// Planted defect this catches: the player made with its autoplay whatever the
// tab, which starts the copy that loads while the user looks at Files.
test('a video whose source arrives while its tab is hidden does not start, and waits for Play once shown', async () => {
  const { root, viewer, shown } = mountTab()
  const { medium, answer } = attachedVideo()
  viewer.show(medium)
  await settle()
  shown.value = false
  answer(null)
  await settle()
  const video = root.querySelector('video')!
  expect([video.getAttribute('src'), video.autoplay, video.paused]).toEqual(['/blob/clip', false, true])

  shown.value = true
  await settle()
  expect(root.querySelector('video')).toBe(video)
  expect(video.paused).toBe(true)
  await video.play()
  shown.value = false
  await settle()
  expect(video.paused).toBe(true)
})

// Planted defect this catches: the player made before the file has an
// address, which a web browser fails, so the user returns to "Couldn’t show
// this video" until the file's description arrives.
test('a tab hidden while its video\'s file is asked for shows no failure once shown again, then the file', async () => {
  const { root, viewer, shown } = mountTab()
  const { medium, answer } = attachedVideo()
  const host = hostContents()
  viewer.show(medium)
  await settle()
  shown.value = false
  answer({ path: '/work/clip.webm', contents: host.contents })
  await settle()
  failPlayersWithoutAddress(root)
  await settle()

  shown.value = true
  await settle()
  expect(root.textContent).not.toContain('Couldn’t show this video.')
  host.describe()
  await settle()
  expect(root.textContent).not.toContain('Couldn’t show this video.')
  expect(root.querySelector('video')?.getAttribute('src')).toBe('/raw?path=/work/clip.webm')
})
