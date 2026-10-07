import { expect, test } from 'bun:test'
import { formatPast } from '../useRelativeTime'

// A message's time against the page's clock, which ticks each second while
// the backend stamps the message: a reply that just arrived is in the past.

test('a moment the page clock has not reached yet reads as just passed, never as to come', () => {
  const now = new Date('2026-10-07T10:30:57.000Z')
  expect(formatPast('2026-10-07T10:30:57.201Z', now)).toBe('a few seconds ago')
  expect(formatPast('2026-10-07T10:25:57.000Z', now)).toBe('5 minutes ago')
})
