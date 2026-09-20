import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { ATTACHMENT_MAX_BYTES } from '../http/attachments'
import { JSON_BODY_MAX_BYTES } from '../http/body-caps'
import { openBackend } from './session'

// A body the backend reads whole has a cap, refused before more than it is
// read (`web-api.md` § Request bodies); the bodies it streams have none,
// which the upload and pipe tests cover.

test('a JSON body over its cap, or an attachment over its own, is refused before it is read', async () => {
  const backend = await openBackend({ dataDir: await mkdtemp(join(tmpdir(), 'demi-bodies-')), port: 0 })
  const tooLarge = async (response: Response) => {
    expect(response.status).toBe(413)
    expect(((await response.json()) as { code: string }).code).toBe('too_large')
  }
  const created = await backend.session.fetch('/api/conversations', {
    method: 'POST',
    body: JSON.stringify({ id: crypto.randomUUID() }),
  })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const title = 'x'.repeat(JSON_BODY_MAX_BYTES)
  await tooLarge(await backend.session.fetch(`/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ title }),
    headers: { 'content-type': 'application/json' },
  }))
  // Without a length up front, the body is counted as it arrives.
  const chunked = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(JSON.stringify({ title })))
      controller.close()
    },
  })
  await tooLarge(await backend.session.fetch(`/api/conversations/${conversation.id}`, {
    method: 'PATCH',
    body: chunked,
    headers: { 'content-type': 'application/json' },
    duplex: 'half',
  } as RequestInit))

  await tooLarge(await backend.session.fetch('/api/attachments', {
    method: 'POST',
    body: new Uint8Array(ATTACHMENT_MAX_BYTES + 1),
    headers: { 'content-type': 'image/png' },
  }))
  const small = await backend.session.fetch('/api/attachments', {
    method: 'POST',
    body: new Uint8Array(2 * JSON_BODY_MAX_BYTES),
    headers: { 'content-type': 'image/png' },
  })
  expect(small.status).toBe(201)

  await backend.close()
}, 60_000)
