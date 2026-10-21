import type { ReplyPicture } from '@demicodes/protocol'
import type { AttachmentLookup, AttachmentOrigin } from '@demicodes/web-ui/markdown/types'
import { galleryBlobSize, galleryBlobUrl, galleryBlobs } from './blobs'
import { WORKSPACE_ROOT } from './workspace'

/** The id of the Host the gallery's conversation runs on, which its agent's uploads record. */
export const GALLERY_HOST = 'host-1'

/**
 * The attachments the gallery's conversation holds, as `demi attachment
 * upload` numbered them (`crates/backend-host-access/src/attachment_commands.rs`), over the
 * gallery's blobs, each with its file's size. The guide, the build, the demo
 * and the chart were uploaded from the workspace's files, the chart from a
 * version its file has changed since, which its viewer does not show: it shows
 * its copy; the release notes from a file the workspace no longer has; the
 * others before uploads recorded their file. a14 and a15 are the copies the
 * backend made of the workspace's pictures that a reply shows
 * (`hostPictureCopies`).
 */
const attachments: Record<string, { name: string; mediaType: string; size: number; blob: string; source?: AttachmentOrigin }> = {
  a1: { name: 'login.png', mediaType: 'image/png', size: 15822, blob: galleryBlobs.screenshot },
  a2: { name: 'checkout.webm', mediaType: 'video/webm', size: 23336, blob: galleryBlobs.recording },
  a3: { name: 'guide.pdf', mediaType: 'application/pdf', size: 734, blob: galleryBlobs.guide, source: { host: GALLERY_HOST, path: `${WORKSPACE_ROOT}/docs/guide.pdf` } },
  a4: { name: 'dashboard-full.png', mediaType: 'image/png', size: 8035, blob: galleryBlobs.fullPage },
  a5: { name: 'timeline.png', mediaType: 'image/png', size: 5883, blob: galleryBlobs.timeline },
  a6: { name: 'chart.png', mediaType: 'image/png', size: 16078, blob: galleryBlobs.chart, source: { host: GALLERY_HOST, path: `${WORKSPACE_ROOT}/assets/photo.png` } },
  a7: { name: 'demo.mp4', mediaType: 'video/mp4', size: 31814, blob: galleryBlobs.unsizedRecording, source: { host: GALLERY_HOST, path: `${WORKSPACE_ROOT}/assets/demo.mp4` } },
  a8: { name: 'settings-light.png', mediaType: 'image/png', size: 1745, blob: galleryBlobs.whiteScreen },
  a10: { name: 'settings-dark.png', mediaType: 'image/png', size: 1753, blob: galleryBlobs.darkScreen },
  a11: { name: 'welcome.mp3', mediaType: 'audio/mpeg', size: 16423, blob: galleryBlobs.voiceOver },
  a12: { name: 'release-notes.md', mediaType: 'text/markdown', size: 496, blob: galleryBlobs.releaseNotes, source: { host: GALLERY_HOST, path: `${WORKSPACE_ROOT}/release-notes.md` } },
  a14: { name: 'photo.png', mediaType: 'image/png', size: 15822, blob: galleryBlobs.screenshot, source: { host: GALLERY_HOST, path: `${WORKSPACE_ROOT}/assets/photo.png` } },
  a15: { name: 'page-full.png', mediaType: 'image/png', size: 8035, blob: galleryBlobs.fullPage, source: { host: GALLERY_HOST, path: `${WORKSPACE_ROOT}/assets/page-full.png` } },
  a13: { name: 'app.zip', mediaType: 'application/zip', size: 140, blob: galleryBlobs.archive, source: { host: GALLERY_HOST, path: `${WORKSPACE_ROOT}/dist/app.zip` } },
}

/**
 * What a gallery reply keeps of the workspace's pictures it shows, as the
 * backend settled each once the reply ended: the copies of the test pattern
 * and the whole login page.
 */
export const hostPictureCopies: ReplyPicture[] = [
  { target: 'assets/photo.png', copy: { state: 'copied', attachment: 'a14' } },
  { target: 'assets/page-full.png', copy: { state: 'copied', attachment: 'a15' } },
]

/**
 * A reply that shows pictures of the workspace: both copied, one still being
 * copied, five the backend could not copy, each for a cause it can give, and
 * one of a reply written before copies were kept.
 */
export const hostPicturesMarkdown = `The sign-in page works again. The test pattern and the whole page, from the workspace:

![The test pattern](assets/photo.png)
![The whole page](assets/page-full.png)

The chart is still being copied:

![The chart](out/chart.png)

These could not be copied, each for its own reason, and the last was written before copies were kept:

![The report](out/report.png)

![The recording](out/recording.png)

![The notes](out/notes.md)

![The capture](out/capture.png)

![The locked file](out/locked.png)

![The old capture](assets/old.png)
`

/** What \`hostPicturesMarkdown\` keeps: the copy of each picture, or why there is none. */
export const hostPicturesKept: ReplyPicture[] = [
  ...hostPictureCopies,
  { target: 'out/chart.png', copy: { state: 'copying' } },
  { target: 'out/report.png', copy: { state: 'failed', reason: { kind: 'missing' } } },
  { target: 'out/recording.png', copy: { state: 'failed', reason: { kind: 'too_large', size: 31_457_280 } } },
  { target: 'out/notes.md', copy: { state: 'failed', reason: { kind: 'not_an_image', mediaType: 'text/markdown' } } },
  { target: 'out/capture.png', copy: { state: 'failed', reason: { kind: 'host_unavailable' } } },
  { target: 'out/locked.png', copy: { state: 'failed', reason: { kind: 'unreadable' } } },
]

/**
 * An attachment of the gallery's conversation, as the product's route
 * answers it: one the conversation does not hold is missing.
 */
export function galleryAttachment(id: string): AttachmentLookup {
  const held = attachments[id]
  if (!held) {
    return { state: 'missing' }
  }
  return {
    state: 'found',
    attachment: {
      name: held.name,
      mediaType: held.mediaType,
      url: galleryBlobUrl(held.blob, held.mediaType),
      size: held.size,
      sha256: held.blob,
      ...galleryBlobSize(held.blob),
      ...(held.source ? { source: held.source } : {}),
    },
  }
}

/**
 * A reply whose list items show what each fix looks like, as agents write
 * them: a screenshot's block on the line after an item's words, which stands
 * on its own line under them as a tile, and two after another item's words,
 * which stand side by side under them.
 */
export const listAttachmentMarkdown = `Two fixes, each with what it looks like now:

- **Text** in the composer is larger (this one is 17 px).
  <attachment id="a5" />
- **Words** in the sidebar no longer break mid-word.
  <attachment id="a1" />
  <attachment id="a6" />

Both are in the branch.
`

/** A reply with one screenshot's block between its words: a tile a third of the message's width, on a line of its own. */
export const loneImageMarkdown = `The sign-in page works again. The form as it shows now:

<attachment id="a1" />

The button waits for the session before it moves on.
`

/**
 * A reply with a screenshot whose background is white and one whose
 * background is dark, side by side: each keeps a faint edge in both themes.
 */
export const edgedImagesMarkdown = `The settings page in the light and the dark appearance:

<attachment id="a8" />
<attachment id="a10" />
`

/** A reply with a recording and the page it ends on, one per line: two tiles side by side. */
export const twoMediaMarkdown = `The recording and the page it ends on:

<attachment id="a2" />
<attachment id="a6" />
`

/**
 * A reply with five pictures one per line: three tiles fill the line and
 * the other two wrap onto the next, the tall page and the wide timeline
 * each fitted whole in its tile.
 */
export const fiveMediaMarkdown = `Each step, from the form to the chart:

<attachment id="a1" />
<attachment id="a4" />
<attachment id="a5" />
<attachment id="a6" />
<attachment id="a2" />
`

/** A reply with a voice-over the agent made: it plays where it is, in its inline player. */
export const audioAttachmentMarkdown = `The voice-over for the intro, in Coral's voice:

<attachment id="a11" />
`

/**
 * A reply that delivers files no preview in the reply shows: a card each,
 * stacked, that opens the file in the work panel. It also holds:
 *
 * - the tag in a sentence and a link to an attachment, which are text;
 * - a number the conversation does not have;
 * - a recording whose size its record does not carry.
 */
export const attachmentMarkdown = `The setup guide, the release notes and the build:

<attachment id="a3" />
<attachment id="a12" />
<attachment id="a13" />

Written in a sentence, <attachment id="a3" /> is text, and so is [the guide](attachment:a3).

<attachment id="a9" />

The demo on its own:

<attachment id="a7" />
`

/**
 * The frame's last reply, which answers the user's screenshot with the run's
 * pictures and recordings: with the transcript's own, they are the series the
 * Media tab steps through.
 */
export const frameMediaMarkdown = `The run, step by step, with the chart before the fix:

<attachment id="a6" />
<attachment id="a2" />
<attachment id="a1" />
<attachment id="a4" />
<attachment id="a5" />
<attachment id="a8" />
<attachment id="a10" />
<attachment id="a7" />
`
