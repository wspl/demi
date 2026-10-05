import { defineStore } from 'pinia'
import {
  identitySchema,
  setupStatusSchema,
  type Credentials,
  type SetupRequest,
  type UserDto,
} from '../api/generated/web-api'
import { apiRequest, ApiError, jsonBody } from '../api/client'

type User = UserDto
type SessionState =
  | { status: 'checking' }
  /** No account exists yet: the visitor creates the master account. */
  | { status: 'setupNeeded' }
  | {
      status: 'signedOut'
      reason?: 'expired'
      error?: string
    }
  | {
      status: 'signedIn'
      user: User
    }

async function readIdentity(response: Response): Promise<User> {
  const parsed = identitySchema.safeParse(await response.json())
  if (!parsed.success) {
    throw new Error('The server returned an invalid account response.')
  }
  return parsed.data.user
}

/**
 * Whether the instance still needs its master account. An answer that
 * cannot be read leaves the visitor on the sign-in page, which reports what
 * fails next.
 */
async function setupNeeded(signal?: AbortSignal): Promise<boolean> {
  try {
    const response = await apiRequest('/setup', { signal })
    return setupStatusSchema.parse(await response.json()).needed
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
    async restore(signal?: AbortSignal): Promise<void> {
      const previous = this.current
      try {
        const response = await apiRequest('/auth/me', { signal })
        const user = await readIdentity(response)
        signal?.throwIfAborted()
        this.current = {
          status: 'signedIn',
          user,
        }
      } catch (error) {
        signal?.throwIfAborted()
        if (error instanceof ApiError && error.code === 'unauthenticated') {
          if (previous.status === 'checking' && (await setupNeeded(signal))) {
            this.current = { status: 'setupNeeded' }
            return
          }
          this.current = {
            status: 'signedOut',
            reason: previous.status === 'signedIn' ? 'expired' : undefined,
          }
          return
        }
        if (previous.status === 'checking') {
          this.current = {
            status: 'signedOut',
            error: 'Could not check your session. Please try signing in.',
          }
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
      this.current = {
        status: 'signedIn',
        user,
      }
    },
    /**
     * Creates the master account, which signs in. When another visitor
     * created it first, the visitor signs in instead.
     */
    async setUp(
      email: string,
      password: string,
      signal: AbortSignal,
    ): Promise<void> {
      try {
        const response = await apiRequest('/setup', {
          method: 'POST',
          ...jsonBody({
            email,
            password,
          } satisfies SetupRequest),
          signal,
        })
        const user = await readIdentity(response)
        signal.throwIfAborted()
        this.current = {
          status: 'signedIn',
          user,
        }
      } catch (error) {
        if (error instanceof ApiError && error.code === 'already_set_up') {
          this.current = { status: 'signedOut' }
        }
        throw error
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
