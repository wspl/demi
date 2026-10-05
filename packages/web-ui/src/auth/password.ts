/**
 * The rule a password being set follows (`web-api.md` § Account API), shared
 * by every form that sets one, and what each form says when it is broken.
 */
export const PASSWORD_MIN_LENGTH = 8

export const PASSWORD_HINT = `At least ${PASSWORD_MIN_LENGTH} characters.`

export const PASSWORD_TOO_SHORT = `A password has at least ${PASSWORD_MIN_LENGTH} characters.`

export const PASSWORDS_DIFFER = 'The two passwords differ.'

/** A password typed so far that is too short to be set. */
export function isTooShort(password: string): boolean {
  return password.length > 0 && password.length < PASSWORD_MIN_LENGTH
}

/** A confirmation typed so far that differs from the password. */
export function differs(password: string, confirmation: string): boolean {
  return confirmation.length > 0 && confirmation !== password
}
