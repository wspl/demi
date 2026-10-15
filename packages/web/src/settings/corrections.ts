/**
 * The refusals the user corrects in the account forms they filled, in the
 * page's words (`web-api.md` § Resource index): they name no request. One
 * the catalogue words, as `email_taken`, takes null, so it says the
 * catalogue's words in the page's language.
 */
export const PASSWORD_CORRECTIONS = { invalid_credentials: 'That is not your current password.' } as const
export const EMAIL_CORRECTIONS = { ...PASSWORD_CORRECTIONS, email_taken: null } as const
export const CODE_CORRECTIONS = {
  invalid_code: 'That code is wrong or expired. Use the newest one.',
  email_taken: null,
} as const
