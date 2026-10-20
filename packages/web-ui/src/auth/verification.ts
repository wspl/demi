import { t } from './locales/text'
import { ref, watch, type Ref } from 'vue'
import { operations, type Operation } from '../ui/operations'
import type { ShownError } from '../infra/errors'
import type { HandoffHost, HandoffVerifyOutcome } from './handoff'

/** What confirming it is the user needs of a host: the password, and a passkey where the account has one. */
export type VerificationHost = Pick<HandoffHost, 'verifyPassword' | 'verifyPasskey'>

/** The state of a confirmation step: the password typed, what is wrong with it, and which way is under way. */
export interface Verification {
  password: Ref<string>
  /** What is wrong with the password, said under it; null when nothing is. */
  passwordProblem: Ref<string | null>
  /** A failure of the confirmation, said under its fields. */
  error: Ref<ShownError | null>
  /**
   * Continue's and Use Passkey's operations, one group (`operations.ts`):
   * while one works, the other cannot be pressed; Cancel always can.
   */
  withPassword: Operation
  withPasskey: Operation
  /** Starts the step over, as it opens. */
  reset(): void
  submitPassword(): Promise<void>
  usePasskey(): Promise<void>
}

/**
 * The confirmation that Hand Off to Your Phone, an administrator's Sign-In Code…
 * and the actions that ask it share: the password, red with Required or the
 * host's sentence under it, or Use Passkey where the host offers one.
 *
 * A dismissed browser prompt says nothing. `verified` runs once the host
 * confirmed.
 */
/** How many confirmation steps the page has made, which names each one's operations. */
let verifications = 0

export function useVerification(host: () => VerificationHost, verified: () => void | Promise<void>): Verification {
  const password = ref('')
  const passwordProblem = ref<string | null>(null)
  const error = ref<ShownError | null>(null)
  const target = `verification:${++verifications}`
  const withPassword: Operation = { action: 'password', target, group: 'verify' }
  const withPasskey: Operation = { action: 'passkey', target, group: 'verify' }

  watch(password, () => {
    passwordProblem.value = null
  })

  function reset(): void {
    password.value = ''
    passwordProblem.value = null
    error.value = null
  }

  async function settle(outcome: HandoffVerifyOutcome): Promise<void> {
    switch (outcome.kind) {
      case 'verified':
        await verified()
        break
      case 'wrongPassword':
        passwordProblem.value = outcome.message
        break
      case 'dismissed':
        break
      case 'failed':
        error.value = outcome
        break
    }
  }

  /** Continue, or Return in the field: nothing while the group works. */
  async function submitPassword(): Promise<void> {
    if (password.value === '') {
      passwordProblem.value = t('password.required')
      return
    }
    await operations.run(withPassword, async () => {
      error.value = null
      await settle(await host().verifyPassword(password.value))
    })
  }

  async function usePasskey(): Promise<void> {
    const verify = host().verifyPasskey
    if (!verify) {
      return
    }
    await operations.run(withPasskey, async () => {
      error.value = null
      await settle(await verify())
    })
  }

  return { password, passwordProblem, error, withPassword, withPasskey, reset, submitPassword, usePasskey }
}

/**
 * An action that asks Confirm It’s You (`VerifyDialog.vue`) only when its host
 * answers `needsVerification`, as the backend does for a browser not confirmed in
 * the last five minutes. The page shows the dialog while `message` is set, and
 * settles it with the dialog's answer.
 */
export interface Confirmation {
  /** What the open dialog asks the confirmation for; null while it asks none. */
  message: Ref<string | null>
  /**
   * Runs `run`, and again once the user confirmed it is them each time its
   * answer says that is needed; null when they did not.
   */
  confirmed<T extends { kind: string }>(
    run: () => Promise<T>,
    message: string,
  ): Promise<Exclude<T, { kind: 'needsVerification' }> | null>
  /** The dialog's answer: `verified` once the host confirmed, else the dialog closed. */
  settle(confirmed: boolean): void
}

export function useConfirmation(): Confirmation {
  const message = ref<string | null>(null)
  let pending: ((confirmed: boolean) => void) | null = null

  function ask(asked: string): Promise<boolean> {
    return new Promise((done) => {
      pending = done
      message.value = asked
    })
  }

  function settle(confirmed: boolean): void {
    const done = pending
    pending = null
    message.value = null
    done?.(confirmed)
  }

  async function confirmed<T extends { kind: string }>(
    run: () => Promise<T>,
    asked: string,
  ): Promise<Exclude<T, { kind: 'needsVerification' }> | null> {
    let outcome = await run()
    while (outcome.kind === 'needsVerification') {
      if (!(await ask(asked))) {
        return null
      }
      outcome = await run()
    }
    // TypeScript does not narrow a generic union by a loop's condition.
    return outcome as Exclude<T, { kind: 'needsVerification' }>
  }

  return { message, confirmed, settle }
}
