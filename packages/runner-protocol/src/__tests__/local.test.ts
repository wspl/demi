import { expect, test } from 'bun:test'
import {
  LOCAL,
  localFrame,
  localFrames,
  localInvokeSchema,
  localWatchSchema,
  localManageSchema,
} from '../local'

async function decode(...chunks: Uint8Array[]) {
  async function* input() {
    yield* chunks
  }
  return Array.fromAsync(localFrames(input()))
}

test('local framing rejects unknown types and truncated bodies, preserving fragmented bytes', async () => {
  const bytes = new Uint8Array([0, 255, 128])
  const frame = localFrame(LOCAL.frames.stdout, bytes)
  expect(
    await decode(...Array.from(frame, (byte) => new Uint8Array([byte]))),
  ).toEqual([{ type: LOCAL.frames.stdout, body: bytes }])
  await expect(decode(localFrame(255))).rejects.toThrow(
    'unknown local IPC frame',
  )
  for (let length = 1; length < frame.length; length++) {
    await expect(decode(frame.slice(0, length))).rejects.toThrow(
      'truncated local IPC frame',
    )
  }
  const oversized = new Uint8Array(5)
  new DataView(oversized.buffer).setUint32(0, LOCAL.maxFrame + 1)
  await expect(decode(oversized)).rejects.toThrow('exceeds limit')
})

test('local handshakes reject malformed identities and payload structure', () => {
  const id = 'a'.repeat(32)
  const invoke = {
    version: LOCAL.version,
    id,
    context: id,
    root: 'probe',
    argv: ['hello'],
    cwd: '/',
    env: {},
    live: false,
  }
  expect(localInvokeSchema.parse(invoke)).toEqual(invoke)
  for (const value of [
    { ...invoke, version: 9999 },
    { ...invoke, id: 'bad' },
    { ...invoke, argv: [5] },
    { ...invoke, env: { PATH: 1 } },
    { ...invoke, live: 'false' },
    { ...invoke, unknown: true },
  ])
    expect(localInvokeSchema.safeParse(value).success).toBe(false)
  expect(
    localWatchSchema.safeParse({ version: LOCAL.version, id, context: 1 })
      .success,
  ).toBe(false)
  expect(
    localManageSchema.safeParse({
      version: LOCAL.version,
      secret: id,
      action: 'delete',
    }).success,
  ).toBe(false)
})
