import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { shownError, reportError, type ShownError } from '@demicodes/web-ui/infra/errors'
import type { HandoffHost } from '@demicodes/web-ui/auth/handoff'
import type {
  NewPasswordOutcome,
  SettingsNewUser,
  SettingsUser,
  SettingsUsersHost,
  UserAddOutcome,
  UserRole,
} from '@demicodes/web-ui/settings/types'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import {
  createdUserSchema,
  newFirstPasswordSchema,
  usersSchema,
  type CreateUser,
  type ListedUser,
  type UserChange,
} from '../api/generated/web-api'
import { makeSignInCode, verifyWithPassword, verifyWithPasskey } from '../auth/handoff'
import { useSession } from '../auth/session'
import { browserHasPasskeys } from '../auth/webauthn'
import { useProduct } from '../state/product'

/** A listed account as Settings › Users shows it. */
function row(user: ListedUser): SettingsUser {
  return {
    id: user.id,
    nickname: user.nickname !== '' ? user.nickname : user.email,
    email: user.email,
    role: user.role,
    disabled: user.disabled,
    choosesPassword: user.choosesPassword,
    lastActiveAt: user.lastActiveAt ?? null,
  }
}

/**
 * The accounts as Settings › Users shows and changes them (`web-api.md` §
 * Account API): the list is read from `GET /users` when the page opens and
 * again from each action's answer, since the synchronization channel does not
 * carry it. A refusal the user corrects is said in the dialog that asked; any
 * other is reported.
 */
export const useUserSettings = defineStore('user-settings', () => {
  const session = useSession()
  const product = useProduct()
  const listed = ref<ListedUser[]>([])
  const status = ref<'loading' | 'ready' | 'failed'>('loading')
  const failure = ref<ShownError | null>(null)
  const users = computed(() => listed.value.map(row))

  /** Reads the list; the page shows what it had until the answer comes. */
  async function load(): Promise<void> {
    if (listed.value.length === 0) {
      status.value = 'loading'
    }
    try {
      const response = await apiRequest('/users')
      listed.value = (await readResponse(response, usersSchema)).users
      status.value = 'ready'
      failure.value = null
    } catch (error) {
      if (listed.value.length === 0) {
        status.value = 'failed'
      }
      failure.value = shownError(error)
      reportError('Could Not Load the Users', error, { userVisible: listed.value.length > 0 })
    }
  }

  async function add(user: SettingsNewUser): Promise<UserAddOutcome> {
    try {
      const response = await apiRequest('/users', {
        method: 'POST',
        ...jsonBody({ email: user.email, role: user.role } satisfies CreateUser),
      })
      const created = await readResponse(response, createdUserSchema)
      void load()
      return {
        kind: 'added',
        id: created.user.id,
        nickname: created.user.email,
        password: created.password,
      }
    } catch (error) {
      if (error instanceof ApiError && error.code === 'verification_required') {
        return { kind: 'needsVerification' }
      }
      if (error instanceof ApiError && error.code === 'email_taken') {
        return { kind: 'emailTaken' }
      }
      return { kind: 'failed', ...shownError(error) }
    }
  }

  async function newPassword(id: string): Promise<NewPasswordOutcome> {
    try {
      const response = await apiRequest(`/users/${encodeURIComponent(id)}/password`, { method: 'POST' })
      const made = await readResponse(response, newFirstPasswordSchema)
      void load()
      return { kind: 'made', password: made.password }
    } catch (error) {
      if (error instanceof ApiError && error.code === 'verification_required') {
        return { kind: 'needsVerification' }
      }
      reportError('Could Not Make a New Password', error, { userVisible: true })
      return { kind: 'failed' }
    }
  }

  /** One change of an account, then the list again, whatever the answer was. */
  async function change(id: string, body: UserChange, title: string): Promise<void> {
    try {
      await apiRequest(`/users/${encodeURIComponent(id)}`, { method: 'PATCH', ...jsonBody(body) })
    } catch (error) {
      reportError(title, error, { userVisible: true })
    }
    await load()
  }

  async function setRole(id: string, role: Exclude<UserRole, 'master'>): Promise<void> {
    await change(id, { role }, 'Could Not Change the Role')
  }

  async function setDisabled(id: string, disabled: boolean): Promise<void> {
    await change(id, { disabled }, disabled ? 'Could Not Disable the Account' : 'Could Not Enable the Account')
  }

  async function signOutEverywhere(id: string): Promise<void> {
    try {
      await apiRequest(`/users/${encodeURIComponent(id)}/sign-out`, { method: 'POST' })
    } catch (error) {
      reportError('Could Not Sign the Account Out', error, { userVisible: true })
    }
    await load()
  }

  /** The dialog's host for the account's sign-in codes; the page learns nothing of the account's browsers, so a code is never marked handed off. */
  function signInCode(id: string): HandoffHost {
    return {
      makeCode: () => makeSignInCode(id),
      ...confirmation(),
      handedOffTo: () => null,
    }
  }

  /** The administrator's confirmation: the password, and a passkey where the account has one. */
  function confirmation(): Pick<HandoffHost, 'verifyPassword' | 'verifyPasskey'> {
    const state = product.snapshot
    const passkeys = state?.passkeyRpId && state.passkeys.passkeys.length > 0 && browserHasPasskeys()
    return { verifyPassword: verifyWithPassword, verifyPasskey: passkeys ? verifyWithPasskey : undefined }
  }

  const host = computed<SettingsUsersHost | null>(() =>
    session.user
      ? {
          users: users.value,
          viewerId: session.user.id,
          add,
          newPassword,
          setRole,
          signOutEverywhere,
          setDisabled,
          signInCode,
          ...confirmation(),
        }
      : null,
  )

  return { users, status, failure, host, load }
})
