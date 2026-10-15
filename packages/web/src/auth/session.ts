import { defineStore } from 'pinia'
import { coded, shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import {
  identitySchema,
  setupStatusSchema,
  signInChallengeSchema,
  type Credentials,
  type PasskeySignIn,
  type PasswordChange,
  type SetupRequest,
  type SetupStatus,
  type UserDto,
} from '../api/generated/web-api'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import { signalUnknownPasskey, signWithPasskey } from './webauthn'

type User = UserDto
type SessionState =
  | { status: 'checking' }
  /** No account exists yet: the visitor creates the master account. */
  | { status: 'setupNeeded' }
  /**
   * Setup created the master account and signed it in, and offers it a
   * passkey before the app opens.
   */
  | { status: 'passkeyStep'; user: User }
  /**
   * The account is signed in and still to choose its own password, which
   * nothing else of the product opens before (`product.md` § User system):
   * the wizard asks, and `passwordChosen` ends the state.
   */
  | { status: 'choosingPassword'; user: User }
  | {
      status: 'signedOut'
      reason?: 'expired'
    }
  /**
   * The backend answered the first session check with its own failure; the
   * page shows it with Retry, which checks again (`web-application.md`
   * § Page synchronization).
   */
  | { status: 'failed'; error: ShownError }
  | {
      status: 'signedIn'
      user: User
    }

async function readIdentity(response: Response): Promise<User> {
  const parsed = identitySchema.safeParse(await response.json())
  if (!parsed.success) {
    throw coded(new Error('The server returned an invalid account response.'), 'unreadable_response')
  }
  return parsed.data.user
}

/** The state of an account that signed in: the product, or first the choice of its own password. */
function signedInAs(user: User): SessionState {
  return user.choosesPassword ? { status: 'choosingPassword', user } : { status: 'signedIn', user }
}

/**
 * `GET /setup`: whether the instance still needs its master account, and
 * the relying party of its passkeys, if it offers them.
 */
export async function setupStatus(signal?: AbortSignal): Promise<SetupStatus> {
  const response = await apiRequest('/setup', { signal })
  return readResponse(response, setupStatusSchema)
}

/**
 * Whether the instance still needs its master account. An answer that
 * cannot be read leaves the visitor on the sign-in page, which reports what
 * fails next.
 */
async function setupNeeded(signal?: AbortSignal): Promise<boolean> {
  try {
    return (await setupStatus(signal)).needed
  } catch {
    signal?.throwIfAborted()
    return false
  }
}

export const useSession = defineStore('session', {
  state: () => ({ current: { status: 'checking' } as SessionState }),
  getters: {
    signedIn: (state) => state.current.status === 'signedIn',
    user: (state) =>
      state.current.status === 'signedIn' ? state.current.user : null,
  },
  actions: {
    /**
     * Checks who is signed in. A check the backend cannot answer waits for it
     * in the HTTP client; only its 401 signs the page out. The first check,
     * or its retry, that the backend fails otherwise shows the failure.
     */
    async restore(signal?: AbortSignal): Promise<void> {
      const previous = this.current
      const first = previous.status === 'checking' || previous.status === 'failed'
      try {
        const response = await apiRequest('/auth/me', { signal })
        const user = await readIdentity(response)
        signal?.throwIfAborted()
        this.current = signedInAs(user)
      } catch (error) {
        signal?.throwIfAborted()
        if (error instanceof ApiError && error.code === 'unauthenticated') {
          if (first && (await setupNeeded(signal))) {
            this.current = { status: 'setupNeeded' }
            return
          }
          this.current = {
            status: 'signedOut',
            reason: previous.status === 'signedIn' ? 'expired' : undefined,
          }
          return
        }
        if (first) {
          this.current = { status: 'failed', error: shownError(error) }
          return
        }
        throw error
      }
    },
    async signIn(
      email: string,
      password: string,
      signal: AbortSignal,
    ): Promise<void> {
      const response = await apiRequest('/auth/login', {
        method: 'POST',
        ...jsonBody({
          email,
          password,
        } satisfies Credentials),
        signal,
      })
      const user = await readIdentity(response)
      signal.throwIfAborted()
      this.current = signedInAs(user)
    },
    /**
     * Signs in with a passkey, which names the account: in the email
     * field's autofill with `conditional`, otherwise in the browser's own
     * prompt. Answers whether it signed in; false when the user dismissed
     * the prompt or `signal` aborted it. A passkey no account has is told to
     * the password manager before the refusal is thrown.
     */
    async signInWithPasskey(conditional: boolean, signal: AbortSignal): Promise<boolean> {
      const challenge = await readResponse(
        await apiRequest('/auth/passkey/options', { method: 'POST', ...jsonBody({}), signal }),
        signInChallengeSchema,
      )
      const credential = await signWithPasskey(challenge.options, conditional, signal)
      if (!credential) {
        return false
      }
      let response: Response
      try {
        response = await apiRequest('/auth/passkey', {
          method: 'POST',
          ...jsonBody({ challengeId: challenge.challengeId, credential } satisfies PasskeySignIn),
          signal,
        })
      } catch (error) {
        if (error instanceof ApiError && error.code === 'unknown_passkey' && error.credentialId && challenge.options.rpId) {
          signalUnknownPasskey(challenge.options.rpId, error.credentialId)
        }
        throw error
      }
      const user = await readIdentity(response)
      signal.throwIfAborted()
      this.current = signedInAs(user)
      return true
    },
    /**
     * Creates the master account, which signs in: at once, or, with
     * `passkeyStep`, once setup's passkey step ends (`finishSetup`). When
     * another visitor created it first, the visitor signs in instead.
     */
    async setUp(
      nickname: string,
      email: string,
      password: string,
      signal: AbortSignal,
      passkeyStep = false,
    ): Promise<void> {
      try {
        const response = await apiRequest('/setup', {
          method: 'POST',
          ...jsonBody({
            nickname,
            email,
            password,
          } satisfies SetupRequest),
          signal,
        })
        const user = await readIdentity(response)
        signal.throwIfAborted()
        this.current = passkeyStep ? { status: 'passkeyStep', user } : { status: 'signedIn', user }
      } catch (error) {
        if (error instanceof ApiError && error.code === 'already_set_up') {
          this.current = { status: 'signedOut' }
        }
        throw error
      }
    },
    /** Ends setup's passkey step: the app opens, signed in. */
    finishSetup(): void {
      if (this.current.status === 'passkeyStep') {
        this.current = { status: 'signedIn', user: this.current.user }
      }
    },
    /**
     * Chooses the account's own password (`PUT /auth/password` with the
     * new one alone). The account stays in its state until the wizard ends,
     * for the passkey offer that follows; `finishChoosing` opens the product.
     */
    async choosePassword(next: string): Promise<void> {
      if (this.current.status !== 'choosingPassword') {
        return
      }
      await apiRequest('/auth/password', { method: 'PUT', ...jsonBody({ next } satisfies PasswordChange) })
      this.current = { status: 'choosingPassword', user: { ...this.current.user, choosesPassword: false } }
    },
    /** Ends the wizard: the product opens, signed in. */
    finishChoosing(): void {
      if (this.current.status === 'choosingPassword') {
        this.current = { status: 'signedIn', user: { ...this.current.user, choosesPassword: false } }
      }
    },
    async signOut(): Promise<void> {
      try {
        await apiRequest('/auth/logout', { method: 'POST' })
      } catch (error) {
        if (!(error instanceof ApiError && error.code === 'unauthenticated')) {
          throw error
        }
      }
      this.current = { status: 'signedOut' }
    },
  },
})
