import { t } from './locales/text'
import { computed, watch } from 'vue'
import { shownError } from '@demicodes/web-ui/infra/errors'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { PasskeyAddOutcome, PasskeyOfferOutcome, SettingsPasskey } from '@demicodes/web-ui/settings/types'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import {
  creationChallengeSchema,
  passkeyAnswerSchema,
  type NewPasskey,
  type PasskeyDto,
  type PasskeyPatch,
} from '../api/generated/web-api'
import { useProduct, type Answer } from '../state/product'
import { verifyWithPassword } from './handoff'
import { browserHasPasskeys, createPasskey, signalAccount, signalUnknownPasskey } from './webauthn'

/** The answer that leaves the passkeys as `next` makes them of those the page last read. */
function passkeysAnswer(next: (current: PasskeyDto[]) => PasskeyDto[]): Answer {
  return (read) => ({
    type: 'passkeys',
    passkeys: next(read.passkeys.passkeys),
    userHandle: read.passkeys.userHandle,
  })
}

/**
 * Adds a passkey to the signed-in account: the session verified first, with the
 * password when one is given, which the backend checks, else as the last five
 * minutes' sign-in left it; then the browser's prompt, then the new credential,
 * which the backend checks and names. Settings › Account and setup's passkey step
 * add one with the password, the offer after a sign-in without.
 */
export function addPasskey(password: string): Promise<PasskeyAddOutcome>
export function addPasskey(password: string | null): Promise<PasskeyOfferOutcome>
export async function addPasskey(password: string | null): Promise<PasskeyOfferOutcome> {
  if (password !== null) {
    const verified = await verifyWithPassword(password)
    if (verified.kind !== 'verified') {
      return verified
    }
  }
  let challenge
  try {
    const response = await apiRequest('/auth/passkeys/options', { method: 'POST' })
    challenge = await readResponse(response, creationChallengeSchema)
  } catch (error) {
    if (error instanceof ApiError && error.code === 'verification_required') {
      return { kind: 'needsPassword' }
    }
    return { kind: 'failed', ...shownError(error) }
  }
  try {
    const credential = await createPasskey(challenge.options)
    if (credential === 'held') {
      return { kind: 'held', message: t('passkeys.held') }
    }
    if (!credential) {
      return { kind: 'dismissed' }
    }
    const product = useProduct()
    const at = product.sent()
    const response = await apiRequest('/auth/passkeys', {
      method: 'POST',
      ...jsonBody({ challengeId: challenge.challengeId, credential } satisfies NewPasskey),
    })
    const { passkey } = await readResponse(response, passkeyAnswerSchema)
    product.answered(at, passkeysAnswer((current) => [...current, passkey]))
    return { kind: 'added' }
  } catch (error) {
    return { kind: 'failed', ...shownError(error) }
  }
}

/**
 * The account's passkeys as Settings › Account shows and changes them; the
 * synchronization channel's `passkeys` part brings every change to every
 * page, this one's own answers first.
 */
export const usePasskeys = defineStore('passkeys', () => {
  const product = useProduct()
  /** The relying party's id, when the server offers passkeys. */
  const rpId = computed(() => product.snapshot?.passkeyRpId ?? null)
  /** Whether this server and this browser offer passkeys. */
  const offered = computed(() => rpId.value !== null && browserHasPasskeys())
  const passkeys = computed<SettingsPasskey[]>(() =>
    (product.snapshot?.passkeys.passkeys ?? []).map((passkey) => ({
      id: passkey.id,
      name: passkey.name,
      synced: passkey.synced,
      createdAt: passkey.createdAt,
      lastUsedAt: passkey.lastUsedAt ?? null,
    })),
  )

  /** The renames on their way, by passkey: each passkey's go one at a time, in the order the user made them. */
  const writes = new SerialQueue()

  /** Renames a passkey as the name is typed: the row shows it at once, and a refusal gives the old name back. */
  function rename(id: string, name: string): Promise<void> {
    const shown = product.change('passkeys', (state) => ({
      ...state,
      passkeys: {
        ...state.passkeys,
        passkeys: state.passkeys.passkeys.map((passkey) => (passkey.id === id ? { ...passkey, name } : passkey)),
      },
    }))
    return writes
      .run(id, async () => {
        shown.send()
        await apiRequest(`/auth/passkeys/${encodeURIComponent(id)}`, {
          method: 'PATCH',
          ...jsonBody({ name } satisfies PasskeyPatch),
        })
        shown.land()
      })
      .catch((error) => {
        shown.drop()
        reportError(t('passkeys.renameFailed'), error, { userVisible: true })
      })
  }

  async function remove(id: string): Promise<void> {
    const at = product.sent()
    try {
      await apiRequest(`/auth/passkeys/${encodeURIComponent(id)}`, { method: 'DELETE' })
    } catch (error) {
      // A passkey already removed is what was asked: its row leaves all the same.
      if (!(error instanceof ApiError && error.code === 'passkey_not_found')) {
        reportError(t('passkeys.removeFailed'), error, { userVisible: true })
        return
      }
    }
    product.answered(at, passkeysAnswer((current) => current.filter((passkey) => passkey.id !== id)))
    if (rpId.value !== null) {
      signalUnknownPasskey(rpId.value, id)
    }
  }

  return { rpId, offered, passkeys, add: addPasskey, rename, remove }
})

/**
 * Tells the password manager, whenever the page learns them, the account's
 * email and name and the passkeys it accepts: after a sign-in, whose
 * snapshot brings them, an email change and each change of the passkeys, so
 * the manager lists the passkey under the current email and hides the ones
 * removed.
 */
export function usePasskeySignals(): void {
  const product = useProduct()
  watch(
    () => {
      const state = product.snapshot
      return state && state.passkeyRpId
        ? {
            rpId: state.passkeyRpId,
            userHandle: state.passkeys.userHandle,
            email: state.user.email,
            nickname: state.user.nickname,
            passkeys: state.passkeys.passkeys,
          }
        : null
    },
    (account, previous) => {
      if (
        !account ||
        (previous &&
          previous.email === account.email &&
          previous.nickname === account.nickname &&
          previous.passkeys.map((passkey) => passkey.id).join() === account.passkeys.map((passkey) => passkey.id).join())
      ) {
        return
      }
      signalAccount(account, account.email, account.nickname, account.passkeys)
    },
    { immediate: true },
  )
}
