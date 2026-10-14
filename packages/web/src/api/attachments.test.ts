import { afterEach, expect, test } from 'bun:test'
import { computed, ref, watch } from 'vue'
import type { AttachmentLookup } from '@demicodes/web-ui/markdown/types'
import { lookupAttachment } from './attachments'

// A reply's `attachment:a3` shows the attachment the conversation has by
// that number (`file-previews.md` § Files named in messages), and one named
// before it was uploaded shows once the summary announces it. Pure: the
// backend is a stubbed fetch and the summary a ref; a few milliseconds.

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

const BLOB = 'a'.repeat(64)

/** Answers the attachment route from `stored`, by number, and keeps the numbers asked. */
function stubBackend(stored: Set<string>): string[] {
  const asked: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const id = decodeURIComponent(String(input).split('/').at(-1) ?? '')
    asked.push(id)
    return stored.has(id)
      ? Response.json({ id, name: `${id}.png`, mediaType: 'image/png', size: 4, width: 4, height: 3, blob: BLOB })
      : Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `No attachment ${id}` }, { status: 404 })
  }) as typeof fetch
  return asked
}

/** Resolves once `lookup`, read as a render reads it, is in `state`. */
function until(lookup: () => AttachmentLookup, state: AttachmentLookup['state']): Promise<void> {
  const shown = computed(() => lookup().state)
  if (shown.value === state)
    return Promise.resolve()
  return new Promise((resolve) => {
    const stop = watch(shown, (now) => {
      if (now === state) {
        stop()
        resolve()
      }
    })
  })
}

test('a reply that named an attachment before its upload shows it once the summary announces it', async () => {
  const stored = new Set(['a1'])
  const asked = stubBackend(stored)
  const last = ref(1)
  // The earlier reply names a2 by mistake, before anything is uploaded as a2.
  const earlier = () => lookupAttachment('c-1', 'a2', () => last.value)
  await until(earlier, 'missing')

  // `demi attachment upload` stores a2, and the summary carries it.
  stored.add('a2')
  last.value = 2
  await until(earlier, 'found')
  // The next reply names it too, and both show the same attachment.
  const later = lookupAttachment('c-1', 'a2', () => last.value)
  expect(later).toEqual(earlier())
  expect(later.state === 'found' && later.attachment.name).toBe('a2.png')
  // The page asked only once a2 existed, and only once.
  expect(asked).toEqual(['a2'])
  // The deadline only guards against a page that never shows it.
}, 1000)
