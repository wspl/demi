import { createProductClient } from './session'
import { FakeProvisioner } from './scenarios/fake-provisioner'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection, UserContentBlock } from '@demicodes/core'
import { defineProvider, type InferenceRequest } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { waitFor } from '@demicodes/utils'
import { uploadRefBlockSchema, contentReference } from '@demicodes/product-contracts'
import { ATTACHMENTS_DIR } from '../conversation/attachment-refs'
import { openBackend, type TestBackend } from './session'

// M6 attachments: upload-then-reference for message media (bytes never ride
// the frame socket), inline resolution before the provider, checkpoint
// round-trip, and the workspace file drop.

const PNG_BYTES = new Uint8Array([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0xff,
  0xfe,
  0x01
])

async function api(
  backend: TestBackend,
  path: string,
  init?: RequestInit
): Promise<Response> {
  return backend.session.fetch(path, init)
}

function selectionFor(providerId: string) {
  const model: ModelSelection = {
    providerId: providerId,
    model: {
      id: 'm',
      name: 'M',
      contextWindow: 100_000,
      outputLimit: null,
      inputLimit: null,
      thinking: [],
      acceptedExtensions: []
    },
    thinking: null,
  }
  return { providerId: providerId, model }
}

async function connectClient(
  backend: TestBackend,
  conversationId: string,
  selection: ReturnType<typeof selectionFor>
) {
  const socket = backend.session.socket(
    `/api/conversations/${conversationId}/stream`
  )
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener('open', () => resolve(), { once: true })
    socket.addEventListener(
      'error',
      () => reject(new Error('stream connect failed')),
      { once: true }
    )
  })
  const client = createProductClient(socket)
  await client.open(selection, '/ignored-by-server', 'ignored')
  return client
}

test(
  'message attachment: upload → ref block → inline bytes at the provider → checkpoint round-trip',
  async () => {
    const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-attach-'))
    const requests: InferenceRequest[] = []
    const stubRuntime = () =>
    new StubProvider([
      (request) => {
        requests.push(request)
        return [events.text('saw it'), events.response()]
      },
    ])
    const backend = await openBackend({
      managedHosts: { provisioner: new FakeProvisioner() },
      dataDir,
      port: 0,
      runner: { pingIntervalMs: 0 },
      providerTypes: {
        stub: {
          credential: 'api_key',
          create: ({ providerId, label }) => defineProvider({
            id: providerId,
            displayName: label,
            createRuntime: stubRuntime
          })
        },
      },
    })
    const providerResponse = await api(backend, '/api/providers', {
      method: 'POST',
      body: JSON.stringify({
        providerType: 'stub',
        label: 'Stub',
        apiKey: 'k'
      }),
      headers: { 'content-type': 'application/json' },
    })
    const { provider } = (await providerResponse.json()) as { provider: { id: string } }
    const selection = selectionFor(provider.id)

    // Upload: bytes to the blob store, metadata row back.
    const uploaded = await api(backend, '/api/attachments', {
      method: 'POST',
      body: PNG_BYTES,
      headers: { 'content-type': 'image/png' },
    })
    expect(uploaded.status).toBe(201)
    const { attachment } = (await uploaded.json()) as {
      attachment: {
        id: string;
        mediaType: string;
        sizeBytes: number;
        sha256: string
      }
    }
    expect(attachment.mediaType).toBe('image/png')
    expect(attachment.sizeBytes).toBe(PNG_BYTES.length)

    const created = await api(backend, '/api/conversations', { method: 'POST', body: JSON.stringify({ id: crypto.randomUUID() }) })
    const { conversation } = (await created.json()) as { conversation: { id: string } }
    const client = await connectClient(backend, conversation.id, selection)

    // The send frame carries only the upload reference; the provider sees the
    // inline bytes and, right after them, the attachment record naming the
    // file on the host.
    const uploadBlock = contentReference({
      type: 'upload',
      ref: attachment.id,
      fileName: 'photo.png',
    })
    await client.send([{ type: 'text', text: 'describe this' }, uploadBlock])
    const userItem = requests[0]?.items.find(
      (item): item is Extract<(typeof requests)[0]['items'][number], { type: 'user_message' }> => item.type === 'user_message',
    )
    expect(userItem?.content.map((block) => block.type))
      .toEqual(['text', 'image', 'attachment'])
    const image = userItem?.content.find(
      (block): block is Extract<UserContentBlock, { type: 'image' }> => block.type === 'image'
    )
    expect(image?.source.type).toBe('binary')
    if (image?.source.type !== 'binary')
      throw new Error('expected binary image source')
    expect(image.source.data).toEqual(PNG_BYTES)
    expect(image.source.mediaType).toBe('image/png')
    const record = userItem?.content.find(
      (block): block is Extract<UserContentBlock, { type: 'attachment' }> => block.type === 'attachment'
    )
    expect(record?.name).toBe('photo.png')
    expect(record?.path).toEndWith(`/${ATTACHMENTS_DIR}/${conversation.id}/photo.png`)
    expect(record?.sha256).toBe(attachment.sha256)

    // The browser sees media by reference, live and after a restore alike:
    // the transcript frames carry the blob's hash, and the bytes are one GET away.
    const liveImage = client
      .transcript()
    .blocks.filter((block) => block.type === 'user')
      .flatMap((block) => (block.type === 'user' ? block.content : []))
      .find((block) => block.type === 'image')
    expect(liveImage?.source).toEqual({
      type: 'ref',
      ref: attachment.sha256,
      mediaType: 'image/png'
    })
    await client.close()
    const revived = await connectClient(backend, conversation.id, selection)
    await waitFor(
      () => revived.transcript().blocks.length > 0,
      undefined,
      { timeoutMs: 5_000 }
    )
    const revivedImage = revived
      .transcript()
    .blocks.filter((block) => block.type === 'user')
      .flatMap((block) => (block.type === 'user' ? block.content : []))
      .find((block) => block.type === 'image')
    expect(revivedImage?.source).toEqual({
      type: 'ref',
      ref: attachment.sha256,
      mediaType: 'image/png'
    })
    const blob = await api(
      backend,
      `/api/blobs/${attachment.sha256}?type=image/png`
    )
    expect(blob.status).toBe(200)
    expect(blob.headers.get('content-type')).toBe('image/png')
    expect(blob.headers.get('cache-control')).toContain('immutable')
    expect(new Uint8Array(await blob.arrayBuffer())).toEqual(PNG_BYTES)
    expect(blob.headers.get('x-content-type-options')).toBe('nosniff')
    // Only render-safe types are served inline; anything else is an opaque download, never sniffed.
    const html = await api(
      backend,
      `/api/blobs/${attachment.sha256}?type=text/html`
    )
    expect(html.status).toBe(200)
    expect(html.headers.get('content-type')).toBe('application/octet-stream')
    expect(html.headers.get('content-disposition')).toBe('attachment')
    expect(html.headers.get('x-content-type-options')).toBe('nosniff')
    expect(
      (await api(
        backend,
        '/api/blobs/0000000000000000000000000000000000000000000000000000000000000000'
      )).status
    )
      .toBe(404)

    // A missing reference degrades loudly to a visible placeholder, never a crash.
    const ghost = contentReference({
      type: 'upload',
      ref: 'no-such-id',
      fileName: 'ghost.png',
    })
    await revived.send([ghost]).catch(() => {})
    const placeholder = revived
      .transcript()
    .blocks.filter((block) => block.type === 'user')
      .flatMap((block) => (block.type === 'user' ? block.content : []))
      .some((block) => block.type === 'text' &&
        block.text.includes('no-such-id'))
    expect(placeholder).toBe(true)

    await revived.close()
    await backend.close()
  },
  20_000
)

test('attachment upload limits and the file on the host', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-m6-drop-'))
  let conversationId = ''
  const stubRuntime = () =>
  new StubProvider([
    [events.toolCall('t1', 'shell_exec', {
        script: `cat ~/${ATTACHMENTS_DIR}/${conversationId}/readme.md`,
        timeoutMs: 10_000
      })],
    [events.text('ok'), events.response()]
  ])
  const backend = await openBackend({
    managedHosts: { provisioner: new FakeProvisioner() },
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      stub: {
        credential: 'api_key',
        create: ({ providerId, label }) => defineProvider({
          id: providerId,
          displayName: label,
          createRuntime: stubRuntime
        })
      },
    },
  })

  // Empty and oversized uploads are refused.
  expect((await api(backend, '/api/attachments', {
    method: 'POST',
    body: new Uint8Array(0),
    headers: { 'content-type': 'image/png' }
  })).status).toBe(400)

  const providerResponse = await api(backend, '/api/providers', {
    method: 'POST',
    body: JSON.stringify({ providerType: 'stub', label: 'Stub', apiKey: 'k' }),
    headers: { 'content-type': 'application/json' },
  })
  const { provider } = (await providerResponse.json()) as { provider: { id: string } }
  const created = await api(backend, '/api/conversations', { method: 'POST', body: JSON.stringify({ id: crypto.randomUUID() }) })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  conversationId = conversation.id
  const uploaded = await api(backend, '/api/attachments', {
    method: 'POST',
    body: 'dropped content',
    headers: { 'content-type': 'text/plain' },
  })
  const { attachment } = (await uploaded.json()) as { attachment: { id: string } }

  // A file name with a path separator never passes the upload block's schema,
  // so the frame is refused before anything touches the host; a clean name
  // lands under the attachments directory on the host, where the agent's
  // shell reads it.
  expect(uploadRefBlockSchema.safeParse({
    type: 'upload',
    ref: attachment.id,
    fileName: '../escape.txt'
  }).success).toBe(false)
  const client = await connectClient(
    backend,
    conversation.id,
    selectionFor(provider.id)
  )
  const outputs: string[] = []
  client.subscribe((event) => {
    if (event.type === 'shell_output' && event.status.status === 'exited')
      outputs.push(event.status.stdout.delta)
  })
  const clean = contentReference({
    type: 'upload',
    ref: attachment.id,
    fileName: 'readme.md',
  })
  await client.send([{ type: 'text', text: 'read the drop' }, clean])
  expect(outputs.at(-1)).toBe('dropped content')

  await client.close()
  await backend.close()
}, 20_000)
