const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

/** Whether a sign-in or account field is an email address. */
export function isEmail(value: string): boolean {
  return EMAIL.test(value.trim())
}
