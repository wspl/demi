import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { makeHandoffCode, spendHandoffCode, useSessions, verifyWithPassword } from './handoff'

const originalFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
})
afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = originalFetch
})

/**
 * Answers each `<method> <path>` with the next of its bodies and statuses;
 * answers the bodies the requests sent, in order.
 */
function serve(answers: Record<string, [unknown, number][]>): unknown[] {
  const sent: unknown[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const answer = answers[`${init?.method ?? 'GET'} ${String(input)}`]?.shift()
    if (!answer) {
      throw new Error(`unexpected request ${init?.method ?? 'GET'} ${String(input)}`)
    }
    if (init?.body !== undefined) {
      sent.push(JSON.parse(String(init.body)))
    }
    return Response.json(answer[0], { status: answer[1] })
  }) as unknown as typeof fetch
  return sent
}

const code = { id: 'handoff-1', url: 'https://demi.example/handoff#c0de', expiresInMs: 600_000 }

test('a code for the page the user is on asks to confirm first when the backend refuses it', async () => {
  const sent = serve({
    'POST /api/auth/handoff/code': [
      [{ requestId: 'req_test000000000001', code: 'verification_required', message: 'Confirm it is you first' }, 403],
      [code, 200],
    ],
    'POST /api/auth/verify': [
      [{ requestId: 'req_test000000000001', code: 'invalid_credentials', message: 'Wrong password' }, 401],
      [null, 204],
    ],
  })
  expect(await makeHandoffCode('/chat/7f3a')).toEqual({ kind: 'verify' })
  expect(await verifyWithPassword('wrong')).toEqual({ kind: 'wrongPassword', message: 'Wrong password' })
  expect(await verifyWithPassword('right')).toEqual({ kind: 'verified' })
  // The code expires ten minutes after its answer arrives, on the page's clock.
  const before = Date.now()
  const made = await makeHandoffCode('/chat/7f3a')
  const after = Date.now()
  if (made.kind !== 'code') {
    throw new Error(`a code: ${JSON.stringify(made)}`)
  }
  expect({ id: made.code.id, url: made.code.url }).toEqual({ id: code.id, url: code.url })
  expect(made.code.deadline).toBeGreaterThanOrEqual(before + 600_000)
  expect(made.code.deadline).toBeLessThanOrEqual(after + 600_000)
  expect(sent).toEqual([{ to: '/chat/7f3a' }, { password: 'wrong' }, { password: 'right' }, { to: '/chat/7f3a' }])
})

test('the phone goes to the page the code opens, or learns the code expired', async () => {
  const sent = serve({
    'POST /api/auth/handoff': [
      [{ to: '/chat/7f3a' }, 200],
      [{ requestId: 'req_test000000000001', code: 'handoff_expired', message: 'This code has expired.' }, 401],
    ],
  })
  expect(await spendHandoffCode('c0de')).toEqual({ kind: 'handedOff', to: '/chat/7f3a' })
  expect(await spendHandoffCode('c0de')).toEqual({ kind: 'expired' })
  expect(sent).toEqual([{ code: 'c0de' }, { code: 'c0de' }])
})

test('the dialog learns of the phone from the session its code made', () => {
  const product = useProduct()
  const laptop = {
    id: 'session-1',
    name: 'Safari on macOS',
    createdAt: '2026-10-13T08:00:00.000Z',
    lastActiveAt: '2026-10-13T08:00:00.000Z',
    current: true,
    handoffId: null,
  }
  product.start()
  channels.last().connect(productState({ sessions: [laptop] }))
  const host = useSessions().handoffHost(() => '/chat/7f3a')
  expect(host.handedOffTo('handoff-1')).toBeNull()
  const phone = { ...laptop, id: 'session-2', name: 'Safari on iPhone', current: false, handoffId: 'handoff-1' }
  channels.last().send({ type: 'sessions', sessions: [phone, laptop] })
  expect(host.handedOffTo('handoff-1')).toBe('Safari on iPhone')
  expect(useSessions().sessions.map((session) => [session.name, session.current])).toEqual([
    ['Safari on iPhone', false],
    ['Safari on macOS', true],
  ])
})
