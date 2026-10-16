import { t } from './locales/text'
import { computed } from 'vue'
import { shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { HandoffHost, HandoffStart, HandoffVerifyOutcome } from '@demicodes/web-ui/auth/handoff'
import type { SettingsSession } from '@demicodes/web-ui/settings/types'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import {
  handoffAnswerSchema,
  handoffCodeSchema,
  signInChallengeSchema,
  type HandoffRequest,
  type HandoffSpend,
  type PasskeySignIn,
  type VerifyPassword,
} from '../api/generated/web-api'
import { useProduct } from '../state/product'
import { browserHasPasskeys, signWithPasskey } from './webauthn'

/**
 * A code that opens `to`, the page the user is on, on a phone
 * (`web-api-settings.md` § Account API); `verify` when the backend wants the session
 * confirmed first, which its 403 decides.
 */
export function makeHandoffCode(to: string): Promise<HandoffStart> {
  return startCode(() =>
    apiRequest('/auth/handoff/code', { method: 'POST', ...jsonBody({ to } satisfies HandoffRequest) }),
  )
}

/**
 * An administrator's sign-in code for the account `id` (`web-api-settings.md` § Account
 * API): as a handoff code is made, for a session verified in the last five
 * minutes, and opening the product's home.
 */
export function makeSignInCode(id: string): Promise<HandoffStart> {
  return startCode(() => apiRequest(`/users/${encodeURIComponent(id)}/sign-in-code`, { method: 'POST' }))
}

async function startCode(request: () => Promise<Response>): Promise<HandoffStart> {
  try {
    const response = await request()
    const { id, url, expiresInMs } = await readResponse(response, handoffCodeSchema)
    // The time left counts from the answer's arrival on the page's own clock.
    return { kind: 'code', code: { id, url, deadline: Date.now() + expiresInMs } }
  } catch (error) {
    if (error instanceof ApiError && error.code === 'verification_required') {
      return { kind: 'verify' }
    }
    return { kind: 'failed', ...shownError(error) }
  }
}

/** Confirms it is the user with the account's password. */
export async function verifyWithPassword(password: string): Promise<HandoffVerifyOutcome> {
  try {
    await apiRequest('/auth/verify', { method: 'POST', ...jsonBody({ password } satisfies VerifyPassword) })
    return { kind: 'verified' }
  } catch (error) {
    if (error instanceof ApiError && error.code === 'invalid_credentials') {
      return { kind: 'wrongPassword', message: t('handoff.wrongPassword') }
    }
    return { kind: 'failed', ...shownError(error) }
  }
}

/** Confirms it is the user with a passkey of the account, in the browser's own prompt. */
export async function verifyWithPasskey(): Promise<HandoffVerifyOutcome> {
  try {
    const challenge = await readResponse(
      await apiRequest('/auth/verify/passkey/options', { method: 'POST', ...jsonBody({}) }),
      signInChallengeSchema,
    )
    const credential = await signWithPasskey(challenge.options, false, new AbortController().signal)
    if (!credential) {
      return { kind: 'dismissed' }
    }
    await apiRequest('/auth/verify/passkey', {
      method: 'POST',
      ...jsonBody({ challengeId: challenge.challengeId, credential } satisfies PasskeySignIn),
    })
    return { kind: 'verified' }
  } catch (error) {
    return { kind: 'failed', ...shownError(error) }
  }
}

/**
 * How spending a code on the phone went: the page to go to, signed in to
 * the code's account, in the phone's own session or a new one; the
 * code expired, was spent or is unknown; or another failure.
 */
export type HandoffSpent =
  | { kind: 'handedOff'; to: string }
  | { kind: 'expired' }
  | ({ kind: 'failed' } & ShownError)

/** Spends the code a phone's page read from its address (`web-api-settings.md` § Account API). */
export async function spendHandoffCode(code: string): Promise<HandoffSpent> {
  try {
    const response = await apiRequest('/auth/handoff', {
      method: 'POST',
      ...jsonBody({ code } satisfies HandoffSpend),
    })
    return { kind: 'handedOff', to: (await readResponse(response, handoffAnswerSchema)).to }
  } catch (error) {
    if (error instanceof ApiError && error.code === 'handoff_expired') {
      return { kind: 'expired' }
    }
    return { kind: 'failed', ...shownError(error) }
  }
}

/**
 * The account's signed-in browsers (`users.md` § User system), which the
 * synchronization channel's `sessions` part brings, and the dialog that
 * hands off to a phone: it learns of the phone from the session that
 * carries the code, the one the code made or the phone's own.
 */
export const useSessions = defineStore('sessions', () => {
  const product = useProduct()
  const sessions = computed<SettingsSession[]>(() =>
    (product.snapshot?.sessions ?? []).map((session) => ({
      id: session.id,
      name: session.name,
      createdAt: session.createdAt,
      lastActiveAt: session.lastActiveAt,
      current: session.current,
      issuedBy: session.issuedBy ?? null,
    })),
  )

  async function signOut(id: string): Promise<void> {
    const at = product.sent()
    try {
      await apiRequest(`/auth/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' })
    } catch (error) {
      // A browser already signed out is what was asked: its row leaves all the same.
      if (!(error instanceof ApiError && error.code === 'session_not_found')) {
        reportError(t('handoff.signOutFailed'), error, { userVisible: true })
        return
      }
    }
    product.answered(at, (read) => ({
      type: 'sessions',
      sessions: read.sessions.filter((session) => session.id !== id),
    }))
  }

  /** The dialog's host for a code that opens the page `to` names. */
  function handoffHost(to: () => string): HandoffHost {
    const state = product.snapshot
    const passkeys = state?.passkeyRpId && state.passkeys.passkeys.length > 0 && browserHasPasskeys()
    return {
      makeCode: () => makeHandoffCode(to()),
      verifyPassword: verifyWithPassword,
      verifyPasskey: passkeys ? verifyWithPasskey : undefined,
      handedOffTo: (id) => product.snapshot?.sessions.find((session) => session.handoffId === id)?.name ?? null,
    }
  }

  return { sessions, signOut, handoffHost }
})
