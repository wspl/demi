import { afterEach, expect, test } from 'bun:test'
import { createPasskey } from './webauthn'

// A browser without WebAuthn's JSON helpers, as Safari before 18.4 is: the
// page converts the backend's options and the browser's answer itself.

const GLOBALS = ['PublicKeyCredential', 'AuthenticatorAttestationResponse'] as const
const saved = GLOBALS.map((name) => Object.getOwnPropertyDescriptor(globalThis, name))
const savedCredentials = Object.getOwnPropertyDescriptor(navigator, 'credentials')

afterEach(() => {
  GLOBALS.forEach((name, index) => {
    const descriptor = saved[index]
    if (descriptor) {
      Object.defineProperty(globalThis, name, descriptor)
    } else {
      Reflect.deleteProperty(globalThis, name)
    }
  })
  if (savedCredentials) {
    Object.defineProperty(navigator, 'credentials', savedCredentials)
  } else {
    Reflect.deleteProperty(navigator, 'credentials')
  }
})

const bytes = (...values: number[]) => new Uint8Array(values).buffer

/** The bytes a value the browser is handed holds. */
function held(source: BufferSource | undefined): number[] {
  if (source instanceof ArrayBuffer) {
    return [...new Uint8Array(source)]
  }
  if (ArrayBuffer.isView(source)) {
    return [...new Uint8Array(source.buffer, source.byteOffset, source.byteLength)]
  }
  throw new Error('no bytes')
}

class Attestation {
  clientDataJSON = bytes(0x7b, 0x7d)
  attestationObject = bytes(0xa0)
  getAuthenticatorData() {
    return bytes(0xfb, 0xff)
  }
  getTransports() {
    return ['internal', 'hybrid']
  }
  getPublicKey() {
    return null
  }
  getPublicKeyAlgorithm() {
    return -7
  }
}

class Credential {
  id = 'AQI'
  rawId = bytes(1, 2)
  type = 'public-key'
  authenticatorAttachment = 'platform'
  response = new Attestation()
  getClientExtensionResults() {
    return { credProps: { rk: true } }
  }
}

test('without the JSON helpers, the options reach the browser as bytes and its new passkey the backend as base64url', async () => {
  Object.defineProperty(globalThis, 'PublicKeyCredential', { value: Credential, configurable: true })
  Object.defineProperty(globalThis, 'AuthenticatorAttestationResponse', { value: Attestation, configurable: true })
  const asked: CredentialCreationOptions[] = []
  Object.defineProperty(navigator, 'credentials', {
    value: {
      create: async (options: CredentialCreationOptions) => {
        asked.push(options)
        return new Credential()
      },
    },
    configurable: true,
  })

  const created = await createPasskey({
    rp: { id: 'demi.example', name: 'Demi' },
    user: { id: 'AQI', name: 'ana@example.com', displayName: 'Ana' },
    challenge: '-_8-Pw',
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    excludeCredentials: [{ id: 'AwQ', type: 'public-key', transports: ['internal', 'smoke-signal'] }],
    authenticatorSelection: { residentKey: 'required', requireResidentKey: true, userVerification: 'required' },
    attestation: 'none',
  })

  const publicKey = asked[0]?.publicKey
  expect(held(publicKey?.challenge)).toEqual([0xfb, 0xff, 0x3e, 0x3f])
  expect(held(publicKey?.user.id)).toEqual([1, 2])
  expect(publicKey?.excludeCredentials?.map((descriptor) => held(descriptor.id))).toEqual([[3, 4]])
  // A transport this browser does not know is left out.
  expect(publicKey?.excludeCredentials?.[0]?.transports).toEqual(['internal'])
  expect(publicKey?.authenticatorSelection?.residentKey).toBe('required')
  expect(created).toEqual({
    id: 'AQI',
    rawId: 'AQI',
    type: 'public-key',
    response: {
      clientDataJSON: 'e30',
      attestationObject: 'oA',
      authenticatorData: '-_8',
      transports: ['internal', 'hybrid'],
      publicKeyAlgorithm: -7,
    },
    clientExtensionResults: { credProps: { rk: true } },
    authenticatorAttachment: 'platform',
  })
})
