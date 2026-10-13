import { expect, test } from 'bun:test'
import { z } from 'zod'
import { schemaRefusal } from '../refusal'

// A schema's refusal of a received value names where it failed and what was
// expected, never the value: the page reports it to the server's log
// (`backend.md` § Request IDs and the server log). No I/O; under a
// millisecond.

test('a refusal names paths and types, and none of the value it refused', () => {
  const schema = z.strictObject({
    title: z.string(),
    count: z.number().refine((count) => count > 0, 'Expected a positive count'),
    token: z.string().min(40),
  })
  const parsed = schema.safeParse({ title: 7, count: -31337, token: 'sk-live-secret', 'sk-key-name': true })
  expect(parsed.success).toBe(false)
  const refusal = schemaRefusal(parsed.error!)
  expect(refusal).toContain('expected string, received number')
  expect(refusal).toContain('at title')
  expect(refusal).toContain('Expected a positive count')
  expect(refusal).toContain('at count')
  expect(refusal).toContain('1 unrecognized key')
  for (const value of ['-31337', 'sk-live-secret', 'sk-key-name']) {
    expect(refusal).not.toContain(value)
  }
})
