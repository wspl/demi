import { expect, test } from 'bun:test'
import type { AgentMessage } from '@demicodes/protocol'
import { createPageI18n } from '../../i18n/text'
import { receiptOrigin, receiptRow } from '../agent-receipts'

// Cost: pure, a few milliseconds for the file.

const fromAgent = (event: AgentMessage['event'], origin?: string, number = 3): AgentMessage => ({
  id: event.type === 'completion' ? 'subagent:ag-3:1' : 'm1',
  sender: { id: number === 0 ? 'conversation-1' : 'ag-3', number, description: 'Fix the socket liveness', round: 1 },
  recipientId: 'conversation-1',
  timestamp: '2026-10-11T12:00:00.000Z',
  content: 'Done.',
  event,
  ...(origin === undefined ? {} : { origin }),
})

test('a receipt says what happened and names the agent as its object, tagged when it went wrong', () => {
  // A description is an imperative title: as the subject it read wrongly,
  // "Socket liveness fixes and logging completed", so it stands as the object.
  expect(receiptRow(fromAgent({ type: 'message' })))
    .toEqual({ notice: 'Message from agent', title: 'Fix the socket liveness', inline: false, after: '', tag: null })
  expect(receiptRow(fromAgent({ type: 'completion', outcome: 'completed' })))
    .toEqual({ notice: 'Agent finished', title: 'Fix the socket liveness', inline: false, after: '', tag: null })
  expect(receiptRow(fromAgent({ type: 'completion', outcome: 'failed' })))
    .toEqual({ notice: 'Agent failed', title: 'Fix the socket liveness', inline: false, after: '', tag: 'failed' })
  expect(receiptRow(fromAgent({ type: 'completion', outcome: 'aborted' })))
    .toEqual({ notice: 'Agent stopped', title: 'Fix the socket liveness', inline: false, after: '', tag: 'stopped' })
})

test('a message from the main agent, in its child’s transcript, names it after the notice and opens the call that sent it', () => {
  // The root has no description of its own: as the object, "Message from agent: root session" read wrongly.
  const fromMain = fromAgent({ type: 'message' }, 'root-call', 0)
  expect(receiptRow(fromMain)).toEqual({ notice: 'Message from ', title: 'main agent', inline: true, after: '', tag: null })
  expect(receiptOrigin(fromMain)).toEqual({ node: null, blockId: 'root-call' })
})

test('the rows the user’s decisions and Demi make keep their wording and name no agent', () => {
  const plain = (event: AgentMessage['event']): AgentMessage =>
    ({ id: 'p1', recipientId: 'conversation-1', timestamp: '2026-10-11T12:00:00.000Z', content: 'Body.', event })
  expect(receiptRow(plain({ type: 'permission', outcome: 'allowed', action: 'manage skills' })))
    .toEqual({ notice: 'You allowed this conversation to manage skills', title: null, inline: false, after: '', tag: null })
  expect(receiptRow(plain({ type: 'moved', host: 'MacBook Pro', path: '/Users/alex/app', home: '/Users/alex' })))
    .toEqual({ notice: 'You moved this conversation to MacBook Pro · ~/app', title: null, inline: false, after: '', tag: null })
  expect(receiptOrigin(plain({ type: 'move_failed', reason: 'the project no longer exists' }))).toBeNull()
})

test('a row that does not open says all there is in its words: a failed move says why', () => {
  const plain = (event: AgentMessage['event']): AgentMessage =>
    ({ id: 'move-failed:m1', recipientId: 'conversation-1', timestamp: '2026-10-11T12:00:00.000Z', content: 'Demi could not make the move.', event })
  expect(receiptRow(plain({ type: 'move_failed', reason: 'the project no longer exists' })))
    .toEqual({ notice: 'Move failed: the project no longer exists', title: null, inline: false, after: '', tag: null })
})

test('a receipt opens the block it names in its sender’s transcript, the root’s as the conversation’s own', () => {
  // An update names the call that sent it; a completion the child's last answer.
  expect(receiptOrigin(fromAgent({ type: 'message' }, 'send-call')))
    .toEqual({ node: 'ag-3', blockId: 'send-call' })
  expect(receiptOrigin(fromAgent({ type: 'completion', outcome: 'completed' }, 'last-answer')))
    .toEqual({ node: 'ag-3', blockId: 'last-answer' })
  // A round that gave no answer opens the agent's tab alone.
  expect(receiptOrigin(fromAgent({ type: 'completion', outcome: 'aborted' })))
    .toEqual({ node: 'ag-3', blockId: null })
  expect(receiptOrigin(fromAgent({ type: 'message' }, 'root-call', 0)))
    .toEqual({ node: null, blockId: 'root-call' })
})

// A decision and a failed move carry ids and a code, which the page words in
// its language; the English they recorded is for an older record and the
// agent (`localization.md` § Text from the backend).
test('a decision and a failed move are worded from their ids and code in the page’s language', () => {
  const plain = (event: AgentMessage['event']): AgentMessage =>
    ({ id: 'p1', recipientId: 'conversation-1', timestamp: '2026-10-11T12:00:00.000Z', content: 'Body.', event })
  const both = plain({
    type: 'permission', outcome: 'allowed', action: 'manage devices and manage credentials',
    categories: ['host.devices', 'credentials.manage'], credential: null,
  })
  const credential = plain({ type: 'permission', outcome: 'denied', action: 'use the credential “GitHub”', categories: [], credential: 'GitHub' })
  const moveFailed = plain({ type: 'move_failed', reason: 'the conversation is archived', code: 'conversation_archived' })
  try {
    createPageI18n('zh-Hans')
    expect(receiptRow(both).notice).toBe('你已允许此对话执行以下操作：管理设备和管理凭据')
    createPageI18n('ja')
    expect(receiptRow(both).notice).toBe('この会話に次の操作を許可しました: デバイスの管理、認証情報の管理')
    expect(receiptRow(credential).notice).toBe('この会話に次の操作を許可しませんでした: 認証情報「GitHub」の使用')
    expect(receiptRow(moveFailed).notice).toBe('移動できませんでした: この会話はアーカイブされています。先に復元してください。')
  } finally {
    createPageI18n('en')
  }
})
