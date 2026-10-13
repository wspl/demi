/**
 * Handing off to a phone by a QR code (`product.md` § User system): what
 * the dialog shows of a code, and what its host does for it.
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
  | { kind: 'failed'; message: string }

/**
 * How confirming it is the user ended: confirmed; the password refused,
 * said under its field; the browser's passkey prompt dismissed, which says
 * nothing; or failed, said under the step's fields.
 */
export type HandoffVerifyOutcome =
  | { kind: 'verified' }
  | { kind: 'wrongPassword'; message: string }
  | { kind: 'dismissed' }
  | { kind: 'failed'; message: string }

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

/** The time left until `deadline`, at `now`, both on the page's clock, as the dialog says it: `9:05`; null once it passed. */
export function timeLeft(deadline: number, now: number): string | null {
  const seconds = Math.ceil((deadline - now) / 1000)
  if (seconds <= 0) {
    return null
  }
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
