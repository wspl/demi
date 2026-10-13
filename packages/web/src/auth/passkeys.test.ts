import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { addPasskey } from './passkeys'

// Cost: a stubbed fetch and a stubbed browser; a few milliseconds.
const originalFetch = globalThis.fetch
const credentials = Object.getOwnPropertyDescriptor(navigator, 'credentials')
const publicKeyCredential = Object.getOwnPropertyDescriptor(globalThis, 'PublicKeyCredential')
let pinia: ReturnType<typeof createPinia>
/** The requests the page made, in order. */
let requests: string[]

const challenge = {
  challengeId: 'challenge-1',
  options: {
    rp: { id: 'demi.example', name: 'Demi' },
    user: { id: 'AQI', name: 'ana@example.com', displayName: 'Ana' },
    challenge: '-_8',
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    excludeCredentials: [{ id: 'AwQ', type: 'public-key' }],
    authenticatorSelection: { residentKey: 'required', requireResidentKey: true, userVerification: 'required' },
    attestation: 'none',
  },
}

/** A backend whose session is `stale` until the password verifies it, and which refuses `wrong`. */
function backend(answers: { stale: boolean }): void {
  requests = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = `${init?.method ?? 'GET'} ${String(input)}`
    requests.push(request)
    switch (request) {
      case 'POST /api/auth/verify': {
        const { password } = JSON.parse(String(init?.body))
        if (password === 'wrong') {
          return Response.json({ requestId: 'req_test000000000001', code: 'invalid_credentials', message: 'Wrong password' }, { status: 401 })
        }
        answers.stale = false
        return new Response(null, { status: 204 })
      }
      case 'POST /api/auth/passkeys/options':
        return answers.stale
          ? Response.json({ requestId: 'req_test000000000001', code: 'verification_required', message: 'Confirm it is you first' }, { status: 403 })
          : Response.json(challenge)
      default:
        throw new Error(`unexpected request ${request}`)
    }
  }) as unknown as typeof fetch
}

/** A browser whose password manager already holds a passkey of the account, and so refuses the options. */
function managerHoldingOne(): void {
  Object.defineProperty(globalThis, 'PublicKeyCredential', { configurable: true, value: class {} })
  Object.defineProperty(navigator, 'credentials', {
    configurable: true,
    value: { create: async () => Promise.reject(new DOMException('The authenticator was previously registered', 'InvalidStateError')) },
  })
}

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
})

afterEach(() => {
  disposePinia(pinia)
  globalThis.fetch = originalFetch
  for (const [target, name, descriptor] of [
    [navigator, 'credentials', credentials],
    [globalThis, 'PublicKeyCredential', publicKeyCredential],
  ] as const) {
    if (descriptor) {
      Object.defineProperty(target, name, descriptor)
    } else {
      Reflect.deleteProperty(target, name)
    }
  }
})

test('without the password a session that signed in long ago is asked for it, which then verifies the session first', async () => {
  backend({ stale: true })
  expect(await addPasskey(null)).toEqual({ kind: 'needsPassword' })
  expect(await addPasskey('wrong')).toEqual({ kind: 'wrongPassword', message: 'That is not your password.' })
  managerHoldingOne()
  const held = await addPasskey('right')
  expect(held.kind).toBe('held')
  expect(requests).toEqual([
    'POST /api/auth/passkeys/options',
    'POST /api/auth/verify',
    'POST /api/auth/verify',
    'POST /api/auth/passkeys/options',
  ])
})

test('a fresh sign-in adds without the password, and a manager that holds a passkey already is no failure', async () => {
  backend({ stale: false })
  managerHoldingOne()
  expect(await addPasskey(null)).toEqual({
    kind: 'held',
    message: 'This password manager has a passkey for your account already.',
  })
  expect(requests).toEqual(['POST /api/auth/passkeys/options'])
})
