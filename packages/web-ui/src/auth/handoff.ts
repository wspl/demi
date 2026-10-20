import { t } from './locales/text'
import type { ShownError } from '../infra/errors'

/**
 * Handing off to a phone by a QR code: what the dialog shows of a code, and
 * what its host does for it.
 */

/** A code as the dialog shows it: its id, the URL its QR code holds, and when it expires. */
export interface HandoffCodeView {
  id: string
  url: string
  /**
   * When it expires on the page's own clock, in milliseconds since the
   * epoch: the time the answer arrived plus how long the backend said it
   * lasts, never the server's clock compared with the page's.
   */
  deadline: number
}

/**
 * What asking for a code came to: the code; the backend asks the user to
 * confirm it is them first, which the confirmation step does; or a failure
 * the dialog says.
 */
export type HandoffStart =
  | { kind: 'code'; code: HandoffCodeView }
  | { kind: 'verify' }
  | ({ kind: 'failed' } & ShownError)

/**
 * How confirming it is the user ended: confirmed; the password refused,
 * said under its field; the browser's passkey prompt dismissed, which says
 * nothing; or failed, said under the step's fields.
 */
export type HandoffVerifyOutcome =
  | { kind: 'verified' }
  | { kind: 'wrongPassword'; message: string }
  | { kind: 'dismissed' }
  | ({ kind: 'failed' } & ShownError)

/**
 * What the dialog says, which the same code flow words for who shows it and
 * who scans it: Hand Off to Your Phone (the user's own phone), an
 * administrator's Sign-In Code… for an account, and Add User's last step.
 */
export interface HandoffWording {
  /** The dialog's title: a command's name, title style. */
  title: string
  /** What the confirmation step says under the title. */
  confirm: string
  /** The line under the QR code. */
  scan: string
  /** What the dialog says once a browser spent the code. */
  done: (browser: string) => string
}

export const HANDOFF_WORDING: HandoffWording = {
  get title() {
    return t('handoff.title')
  },
  get confirm() {
    return t('handoff.confirm')
  },
  get scan() {
    return t('handoff.scan')
  },
  done: (browser) => t('handoff.done', { browser }),
}

/** An administrator's Sign-In Code… for the account `name`, which signs a phone in to it. */
export function signInCodeWording(name: string): HandoffWording {
  return {
    get title() {
      return t('handoff.codeTitle')
    },
    get confirm() {
      return t('handoff.codeConfirm', { name })
    },
    get scan() {
      return t('handoff.codeScan')
    },
    done: (browser) => t('handoff.codeDone', { browser }),
  }
}

/**
 * The last step of Add User… and Reset Password…, once the account is made
 * or has a new password: `title` names the action, and the password the
 * server made stands beside the code.
 */
export function userReadyWording(title: string, name: string): HandoffWording {
  const code = signInCodeWording(name)
  return {
    get title() {
      return title
    },
    get confirm() {
      return code.confirm
    },
    get scan() {
      return t('handoff.readyScan')
    },
    done: code.done,
  }
}

/** What the dialog's host does: the backend's side of every step. */
export interface HandoffHost {
  /** A new code for the page the user is on, which ends the one before. */
  makeCode: () => Promise<HandoffStart>
  verifyPassword: (password: string) => Promise<HandoffVerifyOutcome>
  /** Present when the account has a passkey and the server offers them. */
  verifyPasskey?: () => Promise<HandoffVerifyOutcome>
  /**
   * The browser the code `id` was handed off to, by its name, once one has
   * spent it, in a new session or in its own; null until then.
   */
  handedOffTo: (id: string) => string | null
}
