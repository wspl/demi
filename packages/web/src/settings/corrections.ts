import { t } from './locales/text'
/**
 * The refusals the user corrects in the account forms they filled, in the
 * page's words: they name no request. One the catalogue words, as
 * `email_taken`, takes null, so it says the catalogue's words in the page's
 * language.
 */
export const PASSWORD_CORRECTIONS = {
  get invalid_credentials() {
    return t('corrections.invalidCredentials')
  },
} as const
export const EMAIL_CORRECTIONS = {
  get invalid_credentials() {
    return PASSWORD_CORRECTIONS.invalid_credentials
  },
  email_taken: null,
} as const
export const CODE_CORRECTIONS = {
  get invalid_code() {
    return t('corrections.invalidCode')
  },
  email_taken: null,
} as const
