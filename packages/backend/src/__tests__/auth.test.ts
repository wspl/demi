import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { createBackend } from '../index'
import { SESSION_COOKIE } from '../http/cookies'
import { LoginLimiter } from '../auth/login-limiter'
import { MASTER, login, openBackend, setupMaster, webSession } from './session'

// M12 checkpoint 1: the login surface — initial setup, the cookie session
// and its sliding expiry, the session gate, lockout, the own password.

const json = (body: unknown, method = 'POST'): RequestInit => ({ method, body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })

test('setup creates the master once and signs it in; every other route wants the cookie', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-auth-setup-'))
  const backend = await createBackend({ dataDir, port: 0, mode: 'shared' })

  expect(await (await fetch(`${backend.url}/api/setup`)).json()).toEqual({ needed: true })
  const anonymous = await fetch(`${backend.url}/api/conversations`)
  expect(anonymous.status).toBe(401)
  expect(await anonymous.json()).toMatchObject({ code: 'unauthenticated' })

  const master = await setupMaster(backend)
  expect(master.user).toMatchObject({ email: MASTER.email, role: 'master' })
  expect(master.cookie).toMatch(new RegExp(`^${SESSION_COOKIE}=[A-Za-z0-9_-]{40,}$`))
  expect(await (await fetch(`${backend.url}/api/setup`)).json()).toEqual({ needed: false })
  const again = await fetch(`${backend.url}/api/setup`, json({ email: 'other@example.test', password: 'other-pass-1' }))
  expect(again.status).toBe(404)
  expect(await again.json()).toMatchObject({ code: 'already_set_up' })

  expect(await (await master.fetch('/api/auth/me')).json()).toEqual({ user: master.user })
  expect((await master.fetch('/api/conversations')).status).toBe(200)

  // A cookie naming no session is refused and cleared.
  const forged = webSession(backend.url, master.user, `${SESSION_COOKIE}=not-a-session`)
  const refused = await forged.fetch('/api/auth/me')
  expect(refused.status).toBe(401)
  expect(refused.headers.get('set-cookie')).toContain(`${SESSION_COOKIE}=;`)

  await backend.close()
})

test('login, lockout after five failures, logout ends the session', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-auth-login-'))
  const backend = await openBackend({ dataDir, port: 0 })

  const wrong = await fetch(`${backend.url}/api/auth/login`, json({ email: MASTER.email, password: 'nope-nope' }))
  expect(wrong.status).toBe(401)
  expect(await wrong.json()).toMatchObject({ code: 'invalid_credentials' })
  expect(wrong.headers.get('set-cookie')).toBeNull()

  const signedIn = await login(backend, MASTER.email, MASTER.password)
  expect(signedIn.user.id).toBe(backend.session.user.id)
  expect(signedIn.cookie).not.toBe(backend.session.cookie)

  // The earlier success cleared the count: five fresh failures lock the name.
  for (let i = 0; i < 5; i += 1) expect((await fetch(`${backend.url}/api/auth/login`, json({ email: MASTER.email, password: 'nope-nope' }))).status).toBe(401)
  const locked = await fetch(`${backend.url}/api/auth/login`, json(MASTER))
  expect(locked.status).toBe(429)
  expect(await locked.json()).toMatchObject({ code: 'too_many_attempts' })
  // The lock never touches sessions already open.
  expect((await signedIn.fetch('/api/auth/me')).status).toBe(200)

  const out = await signedIn.fetch('/api/auth/logout', { method: 'POST' })
  expect(out.status).toBe(204)
  expect(out.headers.get('set-cookie')).toContain(`${SESSION_COOKIE}=;`)
  expect((await signedIn.fetch('/api/auth/me')).status).toBe(401)
  expect((await backend.session.fetch('/api/auth/me')).status).toBe(200)

  await backend.close()
})

test('the limiter forgets a name after the lock window, so sprayed emails do not accumulate', () => {
  let now = 1_000_000
  const limiter = new LoginLimiter({ lockAfter: 3, lockMs: 1_000, now: () => now })
  for (const name of ['a', 'b', 'c']) limiter.failed(name)
  limiter.failed('a')
  limiter.failed('a')
  expect(limiter.locked('a')).toBe(true)
  expect(limiter.size).toBe(3)
  now += 1_000
  expect(limiter.locked('a')).toBe(false)
  limiter.failed('d')
  expect(limiter.size).toBe(1)
})

test('a user changes their own password with the current one in hand', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-auth-password-'))
  const backend = await openBackend({ dataDir, port: 0 })
  const me = backend.session

  expect((await me.fetch('/api/auth/password', json({ current: 'wrong-wrong', next: 'second-pass-2' }, 'PUT'))).status).toBe(401)
  expect((await me.fetch('/api/auth/password', json({ current: MASTER.password, next: 'short' }, 'PUT'))).status).toBe(400)
  expect((await me.fetch('/api/auth/password', json({ current: MASTER.password, next: 'second-pass-2' }, 'PUT'))).status).toBe(204)

  expect((await fetch(`${backend.url}/api/auth/login`, json(MASTER))).status).toBe(401)
  const fresh = await login(backend, MASTER.email, 'second-pass-2')
  expect(fresh.user.id).toBe(me.user.id)

  await backend.close()
})

test('the session slides: a request near the end renews it, silence past the end expires it', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-auth-sliding-'))
  let now = Date.now()
  const backend = await openBackend({ dataDir, port: 0, auth: { ttlMs: 1000, renewBelowMs: 500, now: () => now } })
  const me = backend.session

  now += 300
  const early = await me.fetch('/api/auth/me')
  expect(early.status).toBe(200)
  expect(early.headers.get('set-cookie')).toBeNull()

  now += 400 // 700 in: under 500 left → renewed to 1700
  const renewed = await me.fetch('/api/auth/me')
  expect(renewed.status).toBe(200)
  expect(renewed.headers.get('set-cookie')).toContain(`${SESSION_COOKIE}=`)

  now += 800 // 1500: past the original end, inside the renewed one
  expect((await me.fetch('/api/auth/me')).status).toBe(200)

  now += 1200 // 2700: silent past 2500
  const expired = await me.fetch('/api/auth/me')
  expect(expired.status).toBe(401)
  expect(expired.headers.get('set-cookie')).toContain(`${SESSION_COOKIE}=;`)

  await backend.close()
})

test('email identity is validated and case insensitive; nickname changes persist separately', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-email-identity-'))
  const backend = await openBackend({ dataDir, port: 0 })
  try {
    const signedIn = await login(backend, `  ${MASTER.email.toUpperCase()}  `, MASTER.password)
    expect(signedIn.user.email).toBe(MASTER.email)
    expect((await fetch(`${backend.url}/api/auth/login`, json({ email: 'invalid', password: MASTER.password }))).status).toBe(400)
    const updated = await signedIn.fetch('/api/auth/me', json({ nickname: '  New name  ' }, 'PATCH'))
    expect(await updated.json()).toMatchObject({ user: { email: MASTER.email, nickname: 'New name' } })
    expect(await (await backend.session.fetch('/api/auth/me')).json()).toMatchObject({ user: { nickname: 'New name' } })
    expect((await signedIn.fetch('/api/auth/me', json({ nickname: ' ', role: 'admin' }, 'PATCH'))).status).toBe(400)
    expect((await signedIn.fetch('/api/auth/email', json({ email: 'next@example.test', password: MASTER.password }))).status).toBe(503)
  } finally {
    await backend.close()
  }
})

test('email changes require a delivered, unexpired, single-use code and survive restart', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-email-change-'))
  const delivered: Array<{ email: string; code: string; expiresAt: number }> = []
  let now = Date.now()
  const options = { dataDir, port: 0, auth: { now: () => now }, accountMail: { sendVerification: async (message: typeof delivered[number]) => { delivered.push(message) } } }
  let backend = await openBackend(options)
  try {
    const me = backend.session
    expect((await me.fetch('/api/auth/email', json({ email: 'next@example.test', password: 'wrong-password' }))).status).toBe(401)
    const started = await me.fetch('/api/auth/email', json({ email: 'NEXT@example.test', password: MASTER.password }))
    expect(started.status).toBe(202)
    const { challenge } = await started.json() as { challenge: { id: string; email: string } }
    expect(delivered[0]?.email).toBe('next@example.test')
    expect(JSON.stringify(challenge)).not.toContain('code')
    expect((await me.fetch('/api/auth/email', json({ email: 'next@example.test', password: MASTER.password }))).status).toBe(429)
    expect((await me.fetch('/api/auth/email/confirm', json({ id: challenge.id, code: 'abcdef' }))).status).toBe(400)
    await backend.close()
    backend = await openBackend(options)
    const confirmed = await backend.session.fetch('/api/auth/email/confirm', json({ id: challenge.id, code: delivered[0]!.code }))
    expect(confirmed.status).toBe(200)
    expect(await confirmed.json()).toMatchObject({ user: { email: 'next@example.test' } })
    expect((await backend.session.fetch('/api/auth/email/confirm', json({ id: challenge.id, code: delivered[0]!.code }))).status).toBe(400)
    expect((await fetch(`${backend.url}/api/auth/login`, json(MASTER))).status).toBe(401)
    expect((await login(backend, 'next@example.test', MASTER.password)).user.id).toBe(me.user.id)
    now += 60_000
    const expired = await backend.session.fetch('/api/auth/email', json({ email: 'later@example.test', password: MASTER.password }))
    const next = await expired.json() as { challenge: { id: string } }
    now += 10 * 60_000
    expect((await backend.session.fetch('/api/auth/email/confirm', json({ id: next.challenge.id, code: delivered[1]!.code }))).status).toBe(400)
  } finally {
    await backend.close()
  }
})

test('email delivery failure allows retry; five wrong codes and password changes invalidate challenges', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-email-failure-'))
  let failing = true
  let code = ''
  let now = Date.now()
  const backend = await openBackend({ dataDir, port: 0, auth: { now: () => now }, accountMail: {
    sendVerification: async message => {
      if (failing) throw new Error('mail transport failed')
      code = message.code
    },
  } })
  try {
    const start = () => backend.session.fetch('/api/auth/email', json({ email: 'new@example.test', password: MASTER.password }))
    expect((await start()).status).toBe(503)
    failing = false
    const { challenge } = await (await start()).json() as { challenge: { id: string } }
    const wrong = code === '000000' ? '111111' : '000000'
    for (let i = 0; i < 5; i += 1) {
      expect((await backend.session.fetch('/api/auth/email/confirm', json({ id: challenge.id, code: wrong }))).status).toBe(400)
    }
    expect((await backend.session.fetch('/api/auth/email/confirm', json({ id: challenge.id, code }))).status).toBe(400)
    now += 60_000
    const next = await (await start()).json() as { challenge: { id: string } }
    await backend.session.fetch('/api/auth/password', json({ current: MASTER.password, next: 'new-password-2' }, 'PUT'))
    expect((await backend.session.fetch('/api/auth/email/confirm', json({ id: next.challenge.id, code }))).status).toBe(400)
  } finally {
    await backend.close()
  }
})
