import { expect, test } from 'bun:test'
import {
  runnerMessageSchema,
  workerMessageSchema,
} from '../commands/worker-messages'

test('worker envelopes validate IO bytes, identity and exit without duplicating command inputs', () => {
  const bytes = new Uint8Array([0, 255])
  expect(
    workerMessageSchema.parse({
      type: 'output',
      id: 1,
      stream: 'stderr',
      bytes,
    }),
  ).toEqual({ type: 'output', id: 1, stream: 'stderr', bytes })
  for (const value of [
    null,
    {},
    { type: 'unexpected' },
    { type: 'input', id: 0 },
    { type: 'input', id: 1.5 },
    { type: 'error', message: 4 },
    { type: 'output', id: 1, stream: 'file', bytes },
    { type: 'output', id: 1, stream: 'stdout', bytes: [0, 255] },
    { type: 'exit', exitCode: -1 },
    { type: 'exit', exitCode: 256 },
    { type: 'exit', exitCode: 1.5 },
  ])
    expect(workerMessageSchema.safeParse(value).success).toBe(false)
  const run = {
    type: 'run' as const,
    path: '/command.mjs',
    cwd: '/',
    env: { PATH: '/bin' },
    args: { limit: null },
  }
  expect(runnerMessageSchema.parse(run)).toEqual(run)
  for (const value of [
    { ...run, args: [] },
    { ...run, cwd: '' },
    { ...run, env: { PATH: 1 } },
    { type: 'reply', id: 1 },
    { type: 'reply', id: 1, value: [] },
    { type: 'reply', id: 1, value: null, error: 'ambiguous' },
  ])
    expect(runnerMessageSchema.safeParse(value).success).toBe(false)
  expect(
    runnerMessageSchema.parse({ type: 'reply', id: 1, value: null }),
  ).toEqual({ type: 'reply', id: 1, value: null })
})
