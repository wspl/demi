import { expect, test } from 'bun:test'
import { agentFragment, shareToken, shownAgent } from './address'

// The shared page's address (`web-api.md` § Sharing). Pure, under a millisecond.

test('a shared page reads its token from /s/<token> and nothing else', () => {
  expect(shareToken('/s/Qm9iAbc')).toBe('Qm9iAbc')
  expect(shareToken('/s/Qm9iAbc/')).toBe('Qm9iAbc')
  expect(shareToken('/s/')).toBeNull()
  expect(shareToken('/chat/c-1')).toBeNull()
  expect(shareToken('/s/a/b')).toBeNull()
})

test('the fragment names the subagent shown, and goes back to the conversation without one', () => {
  expect(shownAgent(agentFragment(4))).toBe(4)
  expect(shownAgent(agentFragment(null))).toBeNull()
  expect(shownAgent('#agent-0')).toBeNull()
  expect(shownAgent('#agent-x')).toBeNull()
})
