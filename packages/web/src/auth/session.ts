import { defineStore } from 'pinia'
import type { z } from 'zod'
import { identitySchema } from '@demicodes/product-contracts'
import { apiRequest, ApiError, jsonBody } from '../api/client'

type User = z.infer<typeof identitySchema>['user']
type SessionState =
  | { status: 'checking' }
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
        }),
        signal,
      })
      const user = await readIdentity(response)
      signal.throwIfAborted()
      this.current = {
        status: 'signedIn',
        user,
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
