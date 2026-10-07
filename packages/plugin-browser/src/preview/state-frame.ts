/**
 * The preview's state frame (`preview.md` § Page state): a hidden frame this
 * page opens on a preview origin, to read that origin's storage for the
 * agent's browser, or to write a page state into it before the tab's page
 * loads. It runs the page-state codec the agent's browser runs too, and
 * shares the storage of the origin's documents in this page, sessionStorage
 * included.
 */
import { z } from 'zod'
import { PREVIEW_FILES_VERSION, pageStorageSchema, type PageStorage } from '../generated/plugin'

/** How long the frame may take to load and answer. */
const STATE_FRAME_MS = 30_000

const answerSchema = z.union([
  z.object({ storage: pageStorageSchema }),
  z.object({ kept: z.array(z.string()) }),
  z.object({ error: z.string() }),
])

/** Runs `request` in a state frame on `origin`, and answers what the frame answered. */
async function ask(origin: string, request: Record<string, unknown>): Promise<z.infer<typeof answerSchema>> {
  const frame = document.createElement('iframe')
  frame.hidden = true
  frame.title = 'Page state'
  // As the preview's own frames: scripts and the origin's storage, nothing else.
  frame.setAttribute('sandbox', 'allow-scripts allow-same-origin')
  frame.src = `${origin}/__demi/v${PREVIEW_FILES_VERSION}/state.html`
  const channel = new MessageChannel()
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    const loaded = new Promise<void>((resolve) => frame.addEventListener('load', () => resolve(), { once: true }))
    document.body.append(frame)
    const answered = (async () => {
      await loaded
      const answer = new Promise<unknown>((resolve) => {
        channel.port1.onmessage = (event) => resolve(event.data)
      })
      frame.contentWindow?.postMessage({ type: 'demi-preview-state', ...request }, origin, [channel.port2])
      return answer
    })()
    const timedOut = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('The preview took too long to read or write the page’s storage.')), STATE_FRAME_MS)
    })
    return answerSchema.parse(await Promise.race([answered, timedOut]))
  } finally {
    clearTimeout(timer)
    channel.port1.close()
    frame.remove()
  }
}

/** The storage of the preview origin `origin`, which stands for the real origin `real`. */
export async function readState(origin: string, real: string): Promise<PageStorage> {
  const answer = await ask(origin, { mode: 'dump', origin: real })
  if ('error' in answer) {
    throw new Error(answer.error)
  }
  if (!('storage' in answer)) {
    throw new Error('the state frame answered a write to a read')
  }
  return answer.storage
}

/** Writes `storage` into the preview origin `origin`; answers the databases another page kept open. */
export async function writeState(origin: string, storage: PageStorage): Promise<string[]> {
  const answer = await ask(origin, { mode: 'seed', storage })
  if ('error' in answer) {
    throw new Error(answer.error)
  }
  if (!('kept' in answer)) {
    throw new Error('the state frame answered a read to a write')
  }
  return answer.kept
}
