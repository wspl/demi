import { expect, test } from 'bun:test'
import {
  configuredModelSchema,
  configuredModelsSchema,
  conversationClientFrameSchema,
  contentReference,
  emailSchema,
  encodeClientFrame,
  preferencesPatchSchema,
  preferencesSchema,
  providerAuthSchema,
  providerRuntimeSchema,
} from '../index'

const attachment = {
  type: 'attachment' as const,
  name: 'kept.txt',
  path: '/home/.demi/attachments/kept.txt',
  mediaType: 'text/plain',
  sizeBytes: 3,
  sha256: '0'.repeat(64),
}

test('product sends reject backend-authored attachments through every frame branch', () => {
  for (const type of ['send', 'steer'] as const) {
    const frame = { type, messageId: 'm', steerId: 's', content: [attachment] }
    expect(conversationClientFrameSchema.safeParse(frame).success).toBe(false)
    expect(
      conversationClientFrameSchema.safeParse({
        ...frame,
        content: [{ type: 'text', text: '' }],
      }).success,
    ).toBe(true)
  }
})

test('product references are validated and encoded into the declared wire shape', () => {
  for (const reference of [
    { type: 'upload' as const, ref: 'upload-id', fileName: 'image.png' },
    { type: 'remote_file' as const, deviceId: 'device', path: '/work/code.ts' },
  ]) {
    const content = [contentReference(reference)]
    const frame = encodeClientFrame({ type: 'send', messageId: 'm', content })
    expect(frame).toEqual({
      type: 'send',
      messageId: 'm',
      content: [reference],
    })
    expect(
      conversationClientFrameSchema.parse(JSON.parse(JSON.stringify(frame))),
    ).toEqual(frame)
    expect(
      conversationClientFrameSchema.safeParse({
        type: 'send',
        messageId: 'm',
        content,
      }).success,
    ).toBe(false)
  }
  for (const source of [
    { type: 'upload', ref: 'u', fileName: '../escape' },
    { type: 'upload', ref: '', fileName: 'ok' },
    { type: 'remote_file', deviceId: 'd', path: 'relative' },
    { type: 'remote_file', deviceId: 'd', path: '/bad\0path' },
  ]) {
    expect(
      conversationClientFrameSchema.safeParse({
        type: 'send',
        messageId: 'm',
        content: [source],
      }).success,
    ).toBe(false)
  }
})

test('edits retain complete attachment content and refuse unresolved product envelopes', () => {
  const frame = {
    type: 'edit_and_send' as const,
    request: {
      operationId: 'edit',
      targetBlockId: 'user',
      version: { epoch: 'epoch', revision: 1 },
      content: [attachment],
    },
  }
  expect(encodeClientFrame(frame)).toEqual(frame)
  const upload = { type: 'upload' as const, ref: 'u', fileName: 'file.txt' }
  expect(
    conversationClientFrameSchema.safeParse({
      ...frame,
      request: { ...frame.request, content: [upload] },
    }).success,
  ).toBe(false)
  expect(() =>
    encodeClientFrame({
      ...frame,
      request: { ...frame.request, content: [contentReference(upload)] },
    }),
  ).toThrow('Editing requires complete content')
})

test('email normalization and rejection share one input contract', () => {
  expect(emailSchema.parse('  USER@EXAMPLE.COM  ')).toBe('user@example.com')
  for (const value of [
    '',
    'a..b@example.com',
    'a@b..com',
    'no-domain',
    'a@b',
    null,
    [],
  ]) {
    expect(emailSchema.safeParse(value).success).toBe(false)
  }
})

test('preference patches distinguish clear, absence and malformed overrides', () => {
  expect(preferencesPatchSchema.parse({ shortcuts: { new: null } })).toEqual({
    shortcuts: { new: null },
  })
  expect(preferencesPatchSchema.parse({})).toEqual({})
  for (const value of [
    { shortcuts: { new: 'x'.repeat(65) } },
    { appearance: { fontSize: '15' } },
    { appearance: { theme: 'unknown' } },
    { shortcuts: { extra: 'x' } },
  ]) {
    expect(preferencesPatchSchema.safeParse(value).success).toBe(false)
  }
  expect(
    preferencesSchema.safeParse({ appearance: {}, shortcuts: { new: null } })
      .success,
  ).toBe(false)
})

test('configured models share limits and reject lossy or inconsistent metadata', () => {
  const model = {
    id: ' model ',
    displayName: ' Model ',
    contextWindow: 100,
    outputLimit: 50,
    thinkingEfforts: [],
    acceptedExtensions: ['png'],
    fastTier: null,
  }
  expect(configuredModelSchema.parse(model)).toMatchObject({
    id: 'model',
    displayName: 'Model',
  })
  for (const patch of [
    { contextWindow: 10.5 },
    { contextWindow: '100' },
    { outputLimit: 101 },
    { acceptedExtensions: ['invalid'] },
    { fastTier: '' },
    { id: 'x'.repeat(257) },
  ]) {
    expect(
      configuredModelSchema.safeParse({ ...model, ...patch }).success,
    ).toBe(false)
  }
  expect(
    configuredModelsSchema.safeParse([model, { ...model, id: 'model' }])
      .success,
  ).toBe(false)
})

test('provider authentication and runtime statuses keep their distinct contracts', () => {
  expect(providerAuthSchema.safeParse({ status: 'ready' }).success).toBe(false)
  expect(
    providerRuntimeSchema.safeParse({ status: 'authenticated' }).success,
  ).toBe(false)
  expect(
    providerAuthSchema.safeParse({ status: 'authenticated' }).success,
  ).toBe(true)
  expect(providerRuntimeSchema.safeParse({ status: 'ready' }).success).toBe(
    true,
  )
  expect(providerAuthSchema.safeParse({ status: 'error' }).success).toBe(false)
})
