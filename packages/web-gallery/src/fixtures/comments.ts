import type { Block } from '@demicodes/protocol'
import { demoModel } from './blocks'

/** The reply the Comments specimens comment on, as the approved prototype drew it. */
export const COMMENTED_REPLY = `Uploads fail on slow connections because the client gives up long before the server does. The upload request runs under the same 30-second timeout as every other API call, so a 40 MB video on a 3G connection is cut off part of the way through, and the retry starts again from the first byte.

The server already supports resumable uploads: it keeps each received chunk for 24 hours and answers \`HEAD /uploads/:id\` with the offset it has. The client never asks for it.

Three changes fix it:

- **Upload in chunks of 4 MB.** Each chunk gets its own request and its own timeout, so a slow link only has to carry 4 MB in time, not the whole file.
- **Resume from the server's offset.** Before a retry, ask the server how much it has and send only the rest, instead of starting over.
- **Back off between retries.** Wait 1, 2, 4 and then 8 seconds, with some jitter, and stop after five attempts so a dead connection does not drain the battery.`

export const COMMENTED_REPLY_ID = 'comments-reply'

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString()

/** The question and the reply the comments are on. */
export function commentedConversation(): Block[] {
  return [
    {
      type: 'user', id: 'comments-question', turnId: 'comments-turn', createdAt: minutesAgo(14), model: demoModel,
      content: [{ type: 'text', text: 'Uploads from the mobile app fail on slow connections. Why, and what should we change?' }],
      preamble: null,
    },
    { type: 'text', id: COMMENTED_REPLY_ID, createdAt: minutesAgo(13), model: demoModel, text: COMMENTED_REPLY },
  ]
}

/** What each specimen's two comments say, and the words of the reply each quotes. */
export const TWO_COMMENTS = [
  { quote: 'Upload in chunks of 4 MB.', text: '4 MB feels large on 2G. Can we start smaller and grow?' },
  { quote: 'the same 30-second timeout', text: 'Should uploads get their own, longer timeout?' },
] as const
