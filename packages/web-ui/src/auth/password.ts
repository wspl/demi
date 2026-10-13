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

/** The characters a generated password uses: letters and digits without the look-alikes (0 O 1 l I), so it can be read out loud or typed from a screen. */
const GENERATED_ALPHABET = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'

export const GENERATED_PASSWORD_LENGTH = 16

/**
 * A random password for an administrator to hand over (`product.md` § User
 * system). Bytes the alphabet's size does not divide are dropped, so every
 * character is equally likely.
 */
export function generatePassword(): string {
  const limit = 256 - (256 % GENERATED_ALPHABET.length)
  let password = ''
  while (password.length < GENERATED_PASSWORD_LENGTH) {
    for (const byte of crypto.getRandomValues(new Uint8Array(GENERATED_PASSWORD_LENGTH))) {
      if (byte < limit && password.length < GENERATED_PASSWORD_LENGTH) {
        password += GENERATED_ALPHABET[byte % GENERATED_ALPHABET.length]
      }
    }
  }
  return password
}
