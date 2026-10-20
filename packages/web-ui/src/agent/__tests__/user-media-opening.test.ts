import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import type { MediaSource } from '../media-source'

// A picture or a video in the user's own message opens where the agent's do:
// in the page's viewer, the work panel's Media tab, at its place among the
// transcript's pictures, with its size. That holds for a file the user
// attached from the computer, for one Attach Remote File chose on the
// conversation's Host, and for a Host image the message names. A sent message on a page
// of happy-dom, given the transcript's series; a few milliseconds besides the
// page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick } = await import('vue')
const { default: SentMessage } = await import('../message-editor/SentMessage.vue')
const { provideBlobUrl } = await import('../media-source')
const { provideMediaViewer, mediumKey } = await import('../../files/media-viewer')
const { provideMessageFiles } = await import('../../markdown/message-files')
const { provideTranscript } = await import('../edit-selection')
const { capsuleMedium } = await import('../transcript-pictures')
const { contentCapsule } = await import('../message-editor/capsules')
const { ATTACHMENT_MARK: MARK } = await import('../../markdown/user-markdown')
const { HostFiles } = await import('../../files/file-cache')
const { keptContents } = await import('../../files/kept-source')
const { SimulatedWatch } = await import('../../files/memory-source')
const { WatchChannel } = await import('../../files/watch-channel')

type MediaViewerState = ReturnType<typeof provideMediaViewer>

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const blobUrl = (ref: string) => `/blob/${ref}`
/** A picture and a video the user attached from the computer, each with its size in bytes. */
const picture = {
  id: 'p1',
  name: 'sketch.png',
  image: { type: 'ref' as const, ref: 'sketch', mediaType: 'image/png', width: 640, height: 480 },
  reference: { type: 'stored' as const, sha256: 'a'.repeat(64), mediaType: 'image/png', size: 2048 },
}
const video = {
  id: 'v1',
  name: 'demo.webm',
  video: { type: 'ref' as const, ref: 'demo', mediaType: 'video/webm', width: 320, height: 180 },
  reference: { type: 'stored' as const, sha256: 'b'.repeat(64), mediaType: 'video/webm', size: 4096 },
}
/** A picture and a video the user attached from the conversation's Host, `dev-1`, as Attach Remote File names them. */
const hostPicture = contentCapsule('h1', [{ type: 'remote_file', deviceId: 'dev-1', path: '/work/picture.png', host: 'laptop' }])
const hostVideo = contentCapsule('h2', [{ type: 'remote_file', deviceId: 'dev-1', path: '/work/clip.mp4', host: 'laptop' }])
/** A picture on another device, which the conversation's Host does not hold. */
const farPicture = contentCapsule('h3', [{ type: 'remote_file', deviceId: 'dev-2', path: '/far/far.png', host: 'desktop' }])

/** The Host's files, which hold the pictures the message names and attaches. */
const kept = new HostFiles()
const watched = new SimulatedWatch()
const contents = keptContents({
  url: (path) => `/raw?path=${path}`,
  describe: async () => ({ value: { size: 9, modifiedAt: null, version: null }, readAt: watched.readAt() }),
}, { files: kept, watch: new WatchChannel(kept, watched.open) })

function mountMessage(markdown: string): { root: HTMLElement; viewer: MediaViewerState } {
  const root = document.createElement('div')
  document.body.append(root)
  let viewer: MediaViewerState | null = null
  const app = createApp(defineComponent({
    setup() {
      provideBlobUrl(blobUrl)
      viewer = provideMediaViewer({ reveal: () => true })
      const files = { cwd: '/work', contents, device: 'dev-1' }
      provideMessageFiles(() => files)
      const address = (source: MediaSource) => (source.type === 'url' ? source.url : blobUrl(source.ref))
      const series = [
        { kind: 'image' as const, src: '/blob/other', name: 'other.png' },
        capsuleMedium(picture, address, files)!,
        capsuleMedium(video, address, files)!,
        capsuleMedium(hostPicture, address, files)!,
        capsuleMedium(hostVideo, address, files)!,
        { kind: 'image' as const, file: { path: '/work/out/shot.png', contents }, name: 'shot' },
      ]
      expect(capsuleMedium(farPicture, address, files)).toBeNull()
      provideTranscript({
        node: null,
        requests: () => ({ requests: [], requestOf: new Map() }),
        directory: () => null,
        pictures: () => series,
      })
      return () => h(SentMessage, { markdown, attachments: [picture, video, hostPicture, hostVideo] })
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  return { root, viewer: viewer! }
}

// Planted defects this catches: a click on the user's picture or video,
// from the computer or the Host, that does nothing, and a Host image that
// opens in Files in place of the viewer.
test('a user\'s pictures, videos and Host image open in the Media tab at their place in the series', async () => {
  const { root, viewer } = mountMessage(`Look at ${MARK} and ${MARK}, on the Host ${MARK} and ${MARK}, then ![shot](out/shot.png)`)
  // The Host image shows once the page has its file's description.
  for (let turn = 0; turn < 4; turn++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
    await nextTick()
  }
  const capsules = [...root.querySelectorAll<HTMLElement>('[data-attachment] [data-drag-handle], [data-message-image] [data-drag-handle]')]
  expect(capsules).toHaveLength(5)
  const opened = []
  for (const capsule of capsules) {
    capsule.click()
    await nextTick()
    const shown = viewer.shown.value
    opened.push([shown && mediumKey(shown), shown?.bytes ?? null, viewer.position.value, viewer.inPanel.value])
  }
  expect(opened).toEqual([
    ['/blob/sketch', 2048, { at: 2, of: 6 }, true],
    ['/blob/demo', 4096, { at: 3, of: 6 }, true],
    ['file:/work/picture.png', null, { at: 4, of: 6 }, true],
    ['file:/work/clip.mp4', null, { at: 5, of: 6 }, true],
    ['file:/work/out/shot.png', null, { at: 6, of: 6 }, true],
  ])
})
