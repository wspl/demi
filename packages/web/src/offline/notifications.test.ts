import { expect, spyOn, test } from 'bun:test'
import type { PushMessage } from '../api/generated/web-api'
import { notificationOf, productPage, pushedNotification, type NotificationWords } from './notifications'

// How the service worker shows a push, worded from the words the page left it.
// Pure functions; under a millisecond.

const ID = '00000000-0000-4000-8000-000000000001'

const WORDS: NotificationWords = {
  untitled: 'New Conversation',
  turnStopped: 'The turn stopped with an error.',
  saveCredential: 'Save the credential “{name}”?',
  allow: 'Allow this conversation to {action}?',
  reach: 'Allow this conversation to reach {host}?',
  pairing: 'Allow this conversation to pair a computer and reach it?',
  useCredential: 'Allow this conversation to use the credential “{name}”?',
  actions: { skills_manage: 'manage your skills' },
}

function finished(answerStart: string | null): PushMessage {
  return { kind: 'turn_finished', conversationId: ID, title: 'Fix the login test', turn: 'r2', answerStart }
}

test('a finished turn says the start of its answer as plain text, cut to 120 characters, tagged with its turn', () => {
  const answer = `**Fixed.** The \`login\` test waited for the cookie; it now waits for the redirect. ${'More detail. '.repeat(20)}`
  const shown = notificationOf(finished(answer), WORDS)
  expect(shown.title).toBe('Fix the login test')
  expect(shown.options.body!.startsWith('Fixed. The login test waited for the cookie; it now waits for the redirect.')).toBe(true)
  expect(shown.options.body).toHaveLength(120)
  expect(shown.options.body!.endsWith('…')).toBe(true)
  expect(shown.options.tag).toBe(`turn:${ID}:r2`)
  expect(shown.options.data).toEqual({ conversationId: ID })
  // Emphasis beside CJK punctuation is emphasis too, never stars in the text.
  expect(notificationOf(finished('**建议：**一组'), WORDS).options.body).toBe('建议：一组')
  // A turn that ended without text says nothing of an answer.
  expect(notificationOf(finished(null), WORDS).options.body).toBe('')
})

test('a conversation without a title, a failed turn and the agent’s own are worded as the page words them', () => {
  const failed = notificationOf({ kind: 'turn_failed', conversationId: ID, title: null, turn: 'e1' }, WORDS)
  expect([failed.title, failed.options.body]).toEqual(['New Conversation', 'The turn stopped with an error.'])
  const agent = notificationOf({ kind: 'agent', conversationId: ID, id: 'n1', title: 'Deploy finished', body: 'All green' }, WORDS)
  expect([agent.title, agent.options.body, agent.options.tag]).toEqual(['Deploy finished', 'All green', 'agent:n1'])
})

test('a request says what its card asks, a category the page does not word in its declared action', () => {
  const asked = (ask: Extract<PushMessage, { kind: 'permission' }>['ask']) =>
    notificationOf({ kind: 'permission', conversationId: ID, title: 'Skills', request: 'q1', ask }, WORDS).options
  expect(asked({ kind: 'permission', categories: [{ id: 'skills.manage', action: 'manage skills' }], host: null }).body)
    .toBe('Allow this conversation to manage your skills?')
  expect(asked({ kind: 'permission', categories: [{ id: 'web.search', action: 'search the web' }], host: null }).body)
    .toBe('Allow this conversation to search the web?')
  expect(asked({ kind: 'permission', categories: [], host: { kind: 'device', deviceId: 'd1', name: 'build-box' } }).body)
    .toBe('Allow this conversation to reach build-box?')
  expect(asked({ kind: 'credential', name: 'Deploy key' }).body).toBe('Save the credential “Deploy key”?')
  expect(asked({ kind: 'allowance', name: 'Deploy key' }).body).toBe('Allow this conversation to use the credential “Deploy key”?')
  expect(asked({ kind: 'credential', name: 'Deploy key' }).tag).toBe('permission:q1')
})

test('a push whose words cannot be read still shows, in the values it carries', async () => {
  const logged = spyOn(console, 'error').mockImplementation(() => {})
  try {
    const shown = await pushedNotification(finished('Done.'), () => Promise.reject(new DOMException('The cache is gone', 'UnknownError')))
    expect(shown.title).toBe('Fix the login test')
    expect(shown.options?.body).toBe('Done.')
    expect(shown.options?.tag).toBe(`turn:${ID}:r2`)
    // One that does not read shows Demi.
    expect((await pushedNotification({ kind: 'unknown' }, async () => WORDS)).title).toBe('demi')
    expect(logged).toHaveBeenCalledTimes(2)
  } finally {
    logged.mockRestore()
  }
})

test('a click opens the conversation in a page of the product, the focused one first, never a share’s page', () => {
  const page = (path: string, focused = false, visibilityState: DocumentVisibilityState = 'hidden') =>
    ({ url: `https://demi.example${path}`, focused, visibilityState })
  const share = page('/s/abc', true, 'visible')
  const hidden = page('/chat/1')
  const visible = page('/settings', false, 'visible')
  expect(productPage([share, hidden, visible])).toBe(visible)
  expect(productPage([share, hidden])).toBe(hidden)
  const focused = page('/', true, 'visible')
  expect(productPage([visible, focused])).toBe(focused)
  // With only a share's page open, a new window opens.
  expect(productPage([share])).toBeUndefined()
})
