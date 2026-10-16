import { expect, test } from 'bun:test'
import { shareToken } from './address'

// The shared page's address (`web-api-conversations.md` § Sharing). Pure, under a millisecond.

test('a shared page reads its token from /s/<token> and nothing else', () => {
  expect(shareToken('/s/Qm9iAbc')).toBe('Qm9iAbc')
  expect(shareToken('/s/Qm9iAbc/')).toBe('Qm9iAbc')
  expect(shareToken('/s/')).toBeNull()
  expect(shareToken('/chat/c-1')).toBeNull()
  expect(shareToken('/s/a/b')).toBeNull()
})
