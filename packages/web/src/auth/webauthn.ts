/**
 * The browser's side of passkeys (`backend.md` § Authentication and
 * ownership): the options the backend gives, handed to
 * `navigator.credentials`, and the browser's answer as the JSON the backend
 * reads; and the signal methods that tell the password manager what the
 * backend holds. Everything here is feature-detected: a browser without
 * `PublicKeyCredential` is offered no passkeys, and one without a signal
 * method is told nothing.
 */
import { base64urlDecode, base64urlEncode } from '@demicodes/utils'
import type {
  AuthenticationResponseJson,
  CreationOptionsJson,
  DescriptorJson,
  PasskeyDto,
  RegistrationResponseJson,
  RequestOptionsJson,
} from '../api/generated/web-api'

/** Whether this browser has WebAuthn at all. */
export function browserHasPasskeys(): boolean {
  return typeof globalThis.PublicKeyCredential === 'function'
}

/** Whether the browser offers passkeys in a field's autofill. */
export async function autofillOffersPasskeys(): Promise<boolean> {
  return (
    browserHasPasskeys() &&
    'isConditionalMediationAvailable' in PublicKeyCredential &&
    (await PublicKeyCredential.isConditionalMediationAvailable())
  )
}

/**
 * Whether `error` is the browser's own end of the prompt: the user dismissed
 * it, it timed out, or the page aborted it. The browser says nothing more
 * of why, so neither does the page.
 */
export function dismissed(error: unknown): boolean {
  return error instanceof DOMException && (error.name === 'NotAllowedError' || error.name === 'AbortError')
}

// `parseCreationOptionsFromJSON`, `parseRequestOptionsFromJSON` and
// `toJSON()` are in Chrome 129, Firefox 119 and Safari 18.4; Safari before
// 18.4 and Chrome before 129 lack them, so the options and the answer are
// converted here then.

function creationOptions(json: CreationOptionsJson): PublicKeyCredentialCreationOptions {
  if ('parseCreationOptionsFromJSON' in PublicKeyCredential) {
    return PublicKeyCredential.parseCreationOptionsFromJSON(json)
  }
  return {
    rp: json.rp,
    user: { id: base64urlDecode(json.user.id), name: json.user.name, displayName: json.user.displayName },
    challenge: base64urlDecode(json.challenge),
    pubKeyCredParams: json.pubKeyCredParams,
    timeout: json.timeout,
    excludeCredentials: json.excludeCredentials?.map(descriptor),
    authenticatorSelection: json.authenticatorSelection,
    attestation: ATTESTATIONS.find((preference) => preference === json.attestation),
    extensions: json.extensions,
  }
}

function requestOptions(json: RequestOptionsJson): PublicKeyCredentialRequestOptions {
  if ('parseRequestOptionsFromJSON' in PublicKeyCredential) {
    return PublicKeyCredential.parseRequestOptionsFromJSON(json)
  }
  return {
    challenge: base64urlDecode(json.challenge),
    timeout: json.timeout,
    rpId: json.rpId,
    allowCredentials: json.allowCredentials?.map(descriptor),
    userVerification: VERIFICATIONS.find((requirement) => requirement === json.userVerification),
    extensions: json.extensions,
  }
}

const ATTESTATIONS: readonly AttestationConveyancePreference[] = ['direct', 'enterprise', 'indirect', 'none']
const VERIFICATIONS: readonly UserVerificationRequirement[] = ['discouraged', 'preferred', 'required']
const TRANSPORTS: readonly AuthenticatorTransport[] = ['ble', 'hybrid', 'internal', 'nfc', 'usb']

/** A credential's descriptor, with the transports this browser knows of those named. */
function descriptor(json: DescriptorJson): PublicKeyCredentialDescriptor {
  return {
    id: base64urlDecode(json.id),
    type: 'public-key',
    transports: json.transports?.flatMap((named) => TRANSPORTS.filter((transport) => transport === named)),
  }
}

/**
 * Asks the browser for a new passkey; null when the user dismissed the
 * prompt.
 */
export async function createPasskey(options: CreationOptionsJson): Promise<RegistrationResponseJson | null> {
  let credential: Credential | null
  try {
    credential = await navigator.credentials.create({ publicKey: creationOptions(options) })
  } catch (error) {
    if (dismissed(error)) {
      return null
    }
    // The options exclude the account's passkeys: the manager holds one.
    if (error instanceof DOMException && error.name === 'InvalidStateError') {
      throw new Error('This password manager has a passkey for your account already.')
    }
    throw error
  }
  if (!(credential instanceof PublicKeyCredential) || !(credential.response instanceof AuthenticatorAttestationResponse)) {
    throw new Error('The browser gave no passkey.')
  }
  return registrationJson(credential, credential.response)
}

/**
 * Asks the browser to sign `options`' challenge with a passkey: in the
 * email field's autofill with `conditional`, otherwise in its own prompt;
 * null when the user dismissed it or `signal` aborted it.
 */
export async function signWithPasskey(
  options: RequestOptionsJson,
  conditional: boolean,
  signal: AbortSignal,
): Promise<AuthenticationResponseJson | null> {
  let credential: Credential | null
  try {
    credential = await navigator.credentials.get({
      publicKey: requestOptions(options),
      ...(conditional ? { mediation: 'conditional' } : {}),
      signal,
    })
  } catch (error) {
    if (dismissed(error)) {
      return null
    }
    throw error
  }
  if (!(credential instanceof PublicKeyCredential) || !(credential.response instanceof AuthenticatorAssertionResponse)) {
    throw new Error('The browser gave no passkey.')
  }
  return authenticationJson(credential, credential.response)
}

function registrationJson(
  credential: PublicKeyCredential,
  response: AuthenticatorAttestationResponse,
): RegistrationResponseJson {
  const json = 'toJSON' in credential ? credential.toJSON() : null
  if (json && 'attestationObject' in json.response) {
    return { ...json, response: json.response }
  }
  const publicKey = response.getPublicKey()
  return {
    id: credential.id,
    rawId: base64urlEncode(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: base64urlEncode(response.clientDataJSON),
      attestationObject: base64urlEncode(response.attestationObject),
      authenticatorData: base64urlEncode(response.getAuthenticatorData()),
      transports: response.getTransports(),
      publicKeyAlgorithm: response.getPublicKeyAlgorithm(),
      ...(publicKey ? { publicKey: base64urlEncode(publicKey) } : {}),
    },
    clientExtensionResults: extensionResults(credential),
    ...(credential.authenticatorAttachment ? { authenticatorAttachment: credential.authenticatorAttachment } : {}),
  }
}

function authenticationJson(
  credential: PublicKeyCredential,
  response: AuthenticatorAssertionResponse,
): AuthenticationResponseJson {
  const json = 'toJSON' in credential ? credential.toJSON() : null
  if (json && 'signature' in json.response) {
    return { ...json, response: json.response }
  }
  return {
    id: credential.id,
    rawId: base64urlEncode(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: base64urlEncode(response.clientDataJSON),
      authenticatorData: base64urlEncode(response.authenticatorData),
      signature: base64urlEncode(response.signature),
      ...(response.userHandle ? { userHandle: base64urlEncode(response.userHandle) } : {}),
    },
    clientExtensionResults: extensionResults(credential),
    ...(credential.authenticatorAttachment ? { authenticatorAttachment: credential.authenticatorAttachment } : {}),
  }
}

/** The outputs of the extensions the backend asks for, the ones the JSON carries. */
function extensionResults(credential: PublicKeyCredential): RegistrationResponseJson['clientExtensionResults'] {
  const results = credential.getClientExtensionResults()
  return {
    ...(results.appid === undefined ? {} : { appid: results.appid }),
    ...(results.credProps === undefined ? {} : { credProps: results.credProps }),
  }
}

/** Where the signals go: the relying party and the account's user handle. */
export interface PasskeyAccount {
  rpId: string
  userHandle: string
}

/**
 * Tells the password manager the account's current email and name and the
 * passkeys the backend accepts, so it shows the email it signs in with and
 * hides the passkeys the account no longer has.
 */
export function signalAccount(
  account: PasskeyAccount,
  email: string,
  nickname: string,
  passkeys: readonly PasskeyDto[],
): void {
  if (!browserHasPasskeys()) {
    return
  }
  if ('signalCurrentUserDetails' in PublicKeyCredential) {
    void PublicKeyCredential.signalCurrentUserDetails({
      rpId: account.rpId,
      userId: account.userHandle,
      name: email,
      displayName: nickname || email,
    }).catch(ignored)
  }
  if ('signalAllAcceptedCredentials' in PublicKeyCredential) {
    void PublicKeyCredential.signalAllAcceptedCredentials({
      rpId: account.rpId,
      userId: account.userHandle,
      allAcceptedCredentialIds: passkeys.map((passkey) => passkey.id),
    }).catch(ignored)
  }
}

/** Tells the password manager no account has the passkey `credentialId`. */
export function signalUnknownPasskey(rpId: string, credentialId: string): void {
  if (browserHasPasskeys() && 'signalUnknownCredential' in PublicKeyCredential) {
    void PublicKeyCredential.signalUnknownCredential({ rpId, credentialId }).catch(ignored)
  }
}

/**
 * A signal the browser refused, as one that is not the relying party's own
 * page: it changes nothing the user relies on, and the browser says nothing
 * the page could act on.
 */
function ignored(): void {}
