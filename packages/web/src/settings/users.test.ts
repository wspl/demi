import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useSession } from '../auth/session'
import { useProduct } from '../state/product'
import { useUserSettings } from './users'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
/** The requests the page sent, as `METHOD path` with the body it carried. */
let sent: string[]
let channels: ReturnType<typeof playChannels>
/** The answers to give by `METHOD path`; each call takes the first. */
let answers: Record<string, Array<[number, unknown]>>

const master = {
  id: 'zan',
  email: 'zan@example.test',
  nickname: 'Zan',
  role: 'master' as const,
  createdAt: '2026-09-01T00:00:00.000Z',
  disabled: false,
  choosesPassword: false,
}
const bo = { ...master, id: 'bo', email: 'bo@example.test', nickname: '', role: 'user' as const, choosesPassword: true }
const refused = (code: string, status: number): [number, unknown] => [
  status,
  { requestId: 'req_test000000000001', code, message: code },
]

// A few milliseconds: the store over the played channel and canned answers, no backend.
beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  sent = []
  answers = {}
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${String(input)}`
    if (key === 'GET /api/models') {
      return Response.json({ providers: [] })
    }
    sent.push(init?.body === undefined ? key : `${key} ${String(init.body)}`)
    const answer = answers[key]?.shift()
    if (!answer) {
      throw new Error(`unexpected request ${key}`)
    }
    return answer[0] === 204 ? new Response(null, { status: 204 }) : Response.json(answer[1], { status: answer[0] })
  }) as unknown as typeof fetch
  channels = playChannels()
  useSession().current = { status: 'signedIn', user: master }
  useProduct().start()
})
afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

const listed = (...users: Array<typeof master | typeof bo>) => users.map((user) => ({ ...user, lastActiveAt: null }))

test('the list is the channel’s users part, keeps an empty nickname, and follows each message of it', () => {
  channels.last().connect(productState({
    users: [{ ...master, lastActiveAt: '2026-10-13T08:00:00.000Z' }, { ...bo, lastActiveAt: null }],
  }))
  const users = useUserSettings()
  expect(users.users.map((user) => [user.nickname, user.choosesPassword, user.lastActiveAt])).toEqual([
    ['Zan', false, '2026-10-13T08:00:00.000Z'],
    ['', true, null],
  ])
  expect(users.host?.viewerId).toBe('zan')
  // The channel brings a change: no request asks for it.
  channels.last().send({ type: 'users', users: listed(master, { ...bo, choosesPassword: false }) })
  expect(users.users.map((user) => user.choosesPassword)).toEqual([false, false])
  expect(sent).toEqual([])
})

test('Add User sends the email and the role alone, and answers the first password the server made', async () => {
  channels.last().connect(productState())
  answers['POST /api/users'] = [[201, { user: bo, password: 'Ab3dEf6hIj9lMn2pQr5T' }]]
  const outcome = await useUserSettings().host!.add({ email: 'bo@example.test', role: 'user' })
  expect(sent).toEqual(['POST /api/users {"email":"bo@example.test","role":"user"}'])
  expect(outcome).toEqual({ kind: 'added', id: 'bo', nickname: 'bo@example.test', password: 'Ab3dEf6hIj9lMn2pQr5T' })
})

test('an action the session has not been verified for says the administrator must confirm, and a taken address is the dialog’s to say', async () => {
  channels.last().connect(productState())
  const host = useUserSettings().host!
  answers['POST /api/users'] = [refused('verification_required', 403), refused('email_taken', 409)]
  expect(await host.add({ email: 'bo@example.test', role: 'user' })).toEqual({ kind: 'needsVerification' })
  expect(await host.add({ email: 'bo@example.test', role: 'user' })).toEqual({ kind: 'emailTaken' })
  answers['POST /api/users/bo/password'] = [refused('verification_required', 403)]
  expect(await host.newPassword('bo')).toEqual({ kind: 'needsVerification' })
})

test('a change goes to the account’s route and reads nothing again', async () => {
  channels.last().connect(productState())
  answers['PATCH /api/users/bo'] = [[204, null], [204, null]]
  answers['POST /api/users/bo/sign-out'] = [[204, null]]
  const host = useUserSettings().host!
  await host.setRole('bo', 'admin')
  await host.setDisabled('bo', true)
  await host.signOutEverywhere('bo')
  expect(sent).toEqual([
    'PATCH /api/users/bo {"role":"admin"}',
    'PATCH /api/users/bo {"disabled":true}',
    'POST /api/users/bo/sign-out',
  ])
})

test('a channel that carries the account alone tells a waiting page when the password was chosen', () => {
  const channel = channels.last()
  channel.open()
  channel.send({ type: 'user', user: { ...bo } })
  const product = useProduct()
  expect(product.account?.choosesPassword).toBe(true)
  expect(product.snapshot).toBeNull()
  channel.send({ type: 'user', user: { ...bo, choosesPassword: false } })
  expect(product.account?.choosesPassword).toBe(false)
  // The state follows, whole.
  channel.send({ type: 'snapshot', state: productState({ user: { ...bo, choosesPassword: false } }) })
  expect(product.snapshot?.user.id).toBe('bo')
})
