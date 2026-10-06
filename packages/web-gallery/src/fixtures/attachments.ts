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

/** A reply that gives the user what the agent made: a screenshot, a recording and a file, and one number the conversation does not have. */
export const attachmentMarkdown = `The sign-in page works again:

![The fixed sign-in page](attachment:a1)

![The flow from sign-in to the dashboard](attachment:a2)

The [setup guide](attachment:a3) has the steps, and [the screenshot](attachment:a1) opens large.

![The old capture](attachment:a9)
`
