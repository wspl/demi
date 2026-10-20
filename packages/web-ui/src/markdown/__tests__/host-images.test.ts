import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { createApp, effectScope, shallowRef } from 'vue'
import { HostFiles } from '../../files/file-cache'
import { keptContents } from '../../files/kept-source'
import { createMediaViewer, useMediumSource } from '../../files/media-viewer'
import { SimulatedWatch } from '../../files/memory-source'
import { WatchChannel } from '../../files/watch-channel'
import { hostImagePaths, useHostImagesAt } from '../host-images'
import { messageMedia } from '../render'
import type { MessageFiles } from '../types'

// A message's Host image shows the file as it is now: the screenshot a reply
// showed is overwritten, and that reply and a later one naming the same path
// show the new picture, learned from the Host's watch, and so does the media
// viewer opened on it. The web browser keeps the picture it decoded for an
// address for as long as the page lives, so the new version must be a new
// address. Played against a simulated Host's watch and numbers. Cost: no I/O,
// fake timers; a few milliseconds.

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

/** Lets the Host's answers and its watch's messages land: they land in microtasks. */
async function settle(): Promise<void> {
  for (let tick = 0; tick < 20; tick++)
    await Promise.resolve()
}

const PATH = '/w/shots/row-hover.png'

function host() {
  const watched = new SimulatedWatch()
  const kept = new HostFiles()
  const watch = new WatchChannel(kept, watched.open)
  let version = '"v1"'
  const described: string[] = []
  const contents = keptContents({
    url: (path, options = {}) => `/raw?path=${path}${options.version ? `&version=${options.version}` : ''}`,
    async describe(path) {
      described.push(path)
      return { value: { size: 4096, modifiedAt: null, version }, readAt: watched.readAt() }
    },
  }, { files: kept, watch })
  const files: MessageFiles = { cwd: '/w', contents, open: () => {} }
  /** Something on the Host overwrites the file, as a subagent's new screenshot does. */
  const overwrite = (next: string) => {
    version = next
    watched.change(PATH)
  }
  return { files, overwrite, described }
}

/** The page's app, whose context a message's component runs in. */
const app = createApp({})

/** Shows a message naming the image, as a reply on screen does, until its scope stops. */
function message(files: MessageFiles) {
  const scope = effectScope()
  const images = app.runWithContext(() => scope.run(() => useHostImagesAt(() => files.contents, () => hostImagePaths(`The new rows:\n\n![row hover](${PATH})`, files)))!)
  return { src: () => images.value.get(PATH), stop: () => scope.stop() }
}

test('a reply’s Host image shows the new version when the file is overwritten, and so does a later reply naming it', async () => {
  const { files, overwrite } = host()
  const first = message(files)
  expect(first.src()).toEqual({ state: 'loading' })
  await settle()
  expect(first.src()).toEqual({ state: 'shown', src: `/raw?path=${PATH}&version="v1"` })

  overwrite('"v2"')
  await settle()
  expect(first.src()).toEqual({ state: 'shown', src: `/raw?path=${PATH}&version="v2"` })

  // A later reply naming the same path shows the version the page keeps, the new one.
  const second = message(files)
  expect(second.src()).toEqual({ state: 'shown', src: `/raw?path=${PATH}&version="v2"` })
  first.stop()
  second.stop()
})

test('the media viewer opened on a Host image shows the version the message shows, and the next one when the file is overwritten', async () => {
  const { files, overwrite } = host()
  const shown = message(files)
  await settle()
  const [medium] = messageMedia(`![row hover](${PATH})`, files)
  const viewer = createMediaViewer(() => ({ page: { width: 1440, height: 900 }, pixelRatio: 1 }))
  // The transcript's pictures find the one clicked by its file, whatever version shows.
  viewer.show({ ...medium!, row: 'r1' }, () => [{ ...medium!, row: 'r1' }])
  expect(viewer.position.value).toEqual({ at: 1, of: 1 })

  const open = shallowRef(true)
  const scope = effectScope()
  const source = app.runWithContext(() => scope.run(() => useMediumSource(() => viewer.shown.value, () => open.value))!)
  await settle()
  expect(source.src.value).toBe(`/raw?path=${PATH}&version="v1"`)

  overwrite('"v2"')
  await settle()
  expect(source.src.value).toBe(`/raw?path=${PATH}&version="v2"`)
  expect(shown.src()).toEqual({ state: 'shown', src: source.src.value })

  // Closing keeps the picture for the window's leave.
  open.value = false
  await settle()
  expect(source.src.value).toBe(`/raw?path=${PATH}&version="v2"`)
  scope.stop()
  shown.stop()
})

test('a reply opened again after the file changed while no reply showed it shows the new version', async () => {
  const { files, overwrite, described } = host()
  const first = message(files)
  await settle()
  first.stop()

  overwrite('"v2"')
  await settle()
  // Nothing on screen shows it, so nothing reads it.
  expect(described).toEqual([PATH])

  const reopened = message(files)
  await settle()
  expect(reopened.src()).toEqual({ state: 'shown', src: `/raw?path=${PATH}&version="v2"` })
  expect(described).toEqual([PATH, PATH])
  reopened.stop()
})
