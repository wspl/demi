import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { useSession } from '../auth/session'
import { useUserSettings } from './users'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
/** The requests the page sent, as `METHOD path` with the body it carried. */
let sent: string[]
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

// A few milliseconds: the store over canned answers, no backend.
beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  sent = []
  answers = {}
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${String(input)}`
    sent.push(init?.body === undefined ? key : `${key} ${String(init.body)}`)
    const answer = answers[key]?.shift()
    if (!answer) {
      throw new Error(`unexpected request ${key}`)
    }
    return answer[0] === 204 ? new Response(null, { status: 204 }) : Response.json(answer[1], { status: answer[0] })
  }) as unknown as typeof fetch
  useSession().current = { status: 'signedIn', user: master }
})
afterEach(() => {
  disposePinia(pinia)
  globalThis.fetch = realFetch
})

test('the list keeps an account’s nickname, empty until it has one, and says who has not chosen a password', async () => {
  answers['GET /api/users'] = [[200, { users: [{ ...master, lastActiveAt: '2026-10-13T08:00:00.000Z' }, { ...bo, lastActiveAt: null }] }]]
  const users = useUserSettings()
  await users.load()
  expect(users.status).toBe('ready')
  expect(users.users.map((user) => [user.nickname, user.choosesPassword, user.lastActiveAt])).toEqual([
    ['Zan', false, '2026-10-13T08:00:00.000Z'],
    ['', true, null],
  ])
  expect(users.host?.viewerId).toBe('zan')
})

test('Add User sends the email and the role alone, and answers the first password the server made', async () => {
  answers['POST /api/users'] = [[201, { user: bo, password: 'Ab3dEf6hIj9lMn2pQr5T' }]]
  answers['GET /api/users'] = [[200, { users: [master, bo].map((user) => ({ ...user, lastActiveAt: null })) }]]
  const outcome = await useUserSettings().host!.add({ email: 'bo@example.test', role: 'user' })
  expect(sent[0]).toBe('POST /api/users {"email":"bo@example.test","role":"user"}')
  expect(outcome).toEqual({ kind: 'added', id: 'bo', nickname: 'bo@example.test', password: 'Ab3dEf6hIj9lMn2pQr5T' })
})

test('an action the session has not been verified for says the administrator must confirm, and a taken address is the dialog’s to say', async () => {
  const host = useUserSettings().host!
  answers['POST /api/users'] = [refused('verification_required', 403), refused('email_taken', 409)]
  expect(await host.add({ email: 'bo@example.test', role: 'user' })).toEqual({ kind: 'needsVerification' })
  expect(await host.add({ email: 'bo@example.test', role: 'user' })).toEqual({ kind: 'emailTaken' })
  answers['POST /api/users/bo/password'] = [refused('verification_required', 403)]
  expect(await host.newPassword('bo')).toEqual({ kind: 'needsVerification' })
})

test('a change goes to the account’s route and the list is read again from the answer', async () => {
  answers['PATCH /api/users/bo'] = [[204, null], [204, null]]
  answers['POST /api/users/bo/sign-out'] = [[204, null]]
  answers['GET /api/users'] = [[200, { users: [] }], [200, { users: [] }], [200, { users: [] }]]
  const host = useUserSettings().host!
  await host.setRole('bo', 'admin')
  await host.setDisabled('bo', true)
  await host.signOutEverywhere('bo')
  expect(sent).toEqual([
    'PATCH /api/users/bo {"role":"admin"}',
    'GET /api/users',
    'PATCH /api/users/bo {"disabled":true}',
    'GET /api/users',
    'POST /api/users/bo/sign-out',
    'GET /api/users',
  ])
})
