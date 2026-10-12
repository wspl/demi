import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { galleryBlobSize, galleryBlobUrl, galleryBlobs } from './blobs'

/**
 * The attachments the gallery's conversation holds, as `demi attachment
 * upload` numbered them (`commands.md` § Attachment commands), over the
 * gallery's blobs.
 */
const attachments: Record<string, { name: string; mediaType: string; blob: string }> = {
  a1: { name: 'login.png', mediaType: 'image/png', blob: galleryBlobs.screenshot },
  a2: { name: 'checkout.webm', mediaType: 'video/webm', blob: galleryBlobs.recording },
  a3: { name: 'guide.pdf', mediaType: 'application/pdf', blob: galleryBlobs.guide },
  a4: { name: 'dashboard-full.png', mediaType: 'image/png', blob: galleryBlobs.fullPage },
  a5: { name: 'timeline.png', mediaType: 'image/png', blob: galleryBlobs.timeline },
  a6: { name: 'chart.png', mediaType: 'image/png', blob: galleryBlobs.chart },
  a7: { name: 'demo.mp4', mediaType: 'video/mp4', blob: galleryBlobs.unsizedRecording },
  a8: { name: 'settings-light.png', mediaType: 'image/png', blob: galleryBlobs.whiteScreen },
  a10: { name: 'settings-dark.png', mediaType: 'image/png', blob: galleryBlobs.darkScreen },
}

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
      sha256: held.blob,
      ...galleryBlobSize(held.blob),
    },
  }
}

/**
 * A reply whose list items show what each fix looks like, as agents write
 * them: a screenshot on the line after an item's words, which stands on its
 * own line under them as a tile, and two screenshots after another item's
 * words, one per line, which stand side by side under them.
 */
export const listAttachmentMarkdown = `Two fixes, each with what it looks like now:

- **Text** in the composer is larger (this one is 17 px).
![The composer's text at 17 px](attachment:a5)
- **Words** in the sidebar no longer break mid-word.
  ![The sidebar before](attachment:a1)
  ![The sidebar after](attachment:a6)

Both are in the branch.
`

/** A reply with one screenshot between its words: a tile a third of the message's width, on a line of its own. */
export const loneImageMarkdown = `The sign-in page works again. The form as it shows now:

![The fixed sign-in page](attachment:a1)

The button waits for the session before it moves on.
`

/**
 * A reply with a screenshot whose background is white and one whose
 * background is dark, side by side: each keeps a faint edge in both themes.
 */
export const edgedImagesMarkdown = `The settings page in the light and the dark appearance:

![The settings list on white](attachment:a8)
![The settings list on dark](attachment:a10)
`

/** A reply with a recording and the page it ends on, one per line: two tiles side by side. */
export const twoMediaMarkdown = `The recording and the page it ends on:

![The flow from sign-in to the dashboard](attachment:a2)
![The dashboard it ends on](attachment:a6)
`

/**
 * A reply with five pictures one per line: three tiles fill the line and
 * the other two wrap onto the next, the tall page and the wide timeline
 * each fitted whole in its tile.
 */
export const fiveMediaMarkdown = `Each step, from the form to the chart:

![The fixed sign-in page](attachment:a1)
![The whole dashboard, top to bottom](attachment:a4)
![The timeline of the steps](attachment:a5)
![The weekly chart](attachment:a6)
![The flow from sign-in to the dashboard](attachment:a2)
`

/**
 * A reply that names attachments in its words: links to them and to a
 * file, a number the conversation does not have, and a recording whose
 * size its record does not carry.
 */
export const attachmentMarkdown = `The [setup guide](attachment:a3) has the steps, and [the screenshot](attachment:a1) and [the recording](attachment:a2) open large.

![The old capture](attachment:a9)

The demo on its own:

![The demo of the checkout](attachment:a7)
`
