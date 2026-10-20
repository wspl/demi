import { t } from './locales/text'
import { computed } from 'vue'
import { defineStore } from 'pinia'
import { shownError, reportError } from '@demicodes/web-ui/infra/errors'
import type { HandoffHost } from '@demicodes/web-ui/auth/handoff'
import type {
  NewPasswordOutcome,
  SettingsCloudStorage,
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
  type CreateUser,
  type ListedUser,
  type UserChange,
} from '../api/generated/web-api'
import { confirmationHost, makeSignInCode } from '../auth/handoff'
import { useSession } from '../auth/session'
import { useProduct } from '../state/product'

/** A listed account as Settings › Users shows it; one without a nickname is named by its email there. */
function row(user: ListedUser): SettingsUser {
  return {
    id: user.id,
    nickname: user.nickname,
    email: user.email,
    role: user.role,
    disabled: user.disabled,
    choosesPassword: user.choosesPassword,
    lastActiveAt: user.lastActiveAt ?? null,
    cloudStorage: user.cloudStorage,
  }
}

/**
 * The accounts as Settings › Users shows and changes them: the list is the product
 * state's `users` part, which the synchronization channel sends to the master and
 * admins and changes with each add, change, reset, disable, enable, sign-in and
 * password choice, so no action reads it again. A refusal the user corrects is
 * said in the dialog that asked; any other is reported.
 */
export const useUserSettings = defineStore('user-settings', () => {
  const session = useSession()
  const product = useProduct()
  const users = computed(() => (product.snapshot?.users ?? []).map(row))

  async function add(user: SettingsNewUser): Promise<UserAddOutcome> {
    try {
      const response = await apiRequest('/users', {
        method: 'POST',
        ...jsonBody({ email: user.email, role: user.role } satisfies CreateUser),
      })
      const created = await readResponse(response, createdUserSchema)
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
      return { kind: 'made', password: made.password }
    } catch (error) {
      if (error instanceof ApiError && error.code === 'verification_required') {
        return { kind: 'needsVerification' }
      }
      reportError(t('users.newPasswordFailed'), error, { userVisible: true })
      return { kind: 'failed' }
    }
  }

  /** One change of an account; the channel brings the list. */
  async function change(id: string, body: UserChange, title: string): Promise<void> {
    try {
      await apiRequest(`/users/${encodeURIComponent(id)}`, { method: 'PATCH', ...jsonBody(body) })
    } catch (error) {
      reportError(title, error, { userVisible: true })
    }
  }

  async function setRole(id: string, role: Exclude<UserRole, 'master'>): Promise<void> {
    await change(id, { role }, t('users.roleFailed'))
  }

  async function setDisabled(id: string, disabled: boolean): Promise<void> {
    await change(id, { disabled }, disabled ? t('users.disableFailed') : t('users.enableFailed'))
  }

  async function setCloudStorage(id: string, cloudStorage: SettingsCloudStorage): Promise<void> {
    await change(id, { cloudStorage }, t('users.cloudStorageFailed'))
  }

  async function signOutEverywhere(id: string): Promise<void> {
    try {
      await apiRequest(`/users/${encodeURIComponent(id)}/sign-out`, { method: 'POST' })
    } catch (error) {
      reportError(t('users.signOutFailed'), error, { userVisible: true })
    }
  }

  /** The dialog's host for the account's sign-in codes; the page learns nothing of the account's browsers, so a code is never marked handed off. */
  function signInCode(id: string): HandoffHost {
    return {
      makeCode: () => makeSignInCode(id),
      ...confirmationHost(),
      handedOffTo: () => null,
    }
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
          setCloudStorage,
          signInCode,
          ...confirmationHost(),
        }
      : null,
  )

  return { users, host }
})
