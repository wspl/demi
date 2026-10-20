import { t } from './locales/text'

/**
 * The rule a password being set follows, shared by every form that sets one, and what
 * each form says when it is broken.
 */
export const PASSWORD_MIN_LENGTH = 8

/** What a field for a new password says about its rule. */
export function passwordHint(): string {
  return t('password.hint', { count: PASSWORD_MIN_LENGTH })
}

/** What a form says when the password is shorter than the rule. */
export function passwordTooShort(): string {
  return t('password.tooShort', { count: PASSWORD_MIN_LENGTH })
}

/** What a form says when the two passwords typed differ. */
export function passwordsDiffer(): string {
  return t('password.differ')
}

/** A password typed so far that is too short to be set. */
export function isTooShort(password: string): boolean {
  return password.length > 0 && password.length < PASSWORD_MIN_LENGTH
}

/** A confirmation typed so far that differs from the password. */
export function differs(password: string, confirmation: string): boolean {
  return confirmation.length > 0 && confirmation !== password
}
