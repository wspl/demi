import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { galleryBlobUrl, galleryBlobs } from './blobs'

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
    attachment: { name: held.name, mediaType: held.mediaType, url: galleryBlobUrl(held.blob, held.mediaType) },
  }
}

/**
 * A reply that gives the user what the agent made: four screenshots of
 * different sizes in a run, a recording and a screenshot in a run, links to
 * them and to a file, a number the conversation does not have, and a lone
 * image after text.
 */
export const attachmentMarkdown = `The sign-in page works again. Each step, from the form to the chart:

![The fixed sign-in page](attachment:a1)
![The whole dashboard, top to bottom](attachment:a4)
![The timeline of the steps](attachment:a5)
![The weekly chart](attachment:a6)

The recording and the page it ends on:

![The flow from sign-in to the dashboard](attachment:a2)

![The dashboard it ends on](attachment:a6)

The [setup guide](attachment:a3) has the steps, and [the screenshot](attachment:a1) opens large.

![The old capture](attachment:a9)

The chart on its own:

![The weekly chart](attachment:a6)
`
