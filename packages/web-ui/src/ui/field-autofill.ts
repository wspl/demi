/**
 * The attributes that keep browsers and password managers from filling or saving
 * a field.
 *
 * `autocomplete="off"` is honored by Safari and Firefox and ignored by Chrome's
 * password manager for a login-looking field. So each manager's own switch is set
 * as well, from its documentation:
 *
 * - 1Password: `data-1p-ignore`
 *   (https://www.1password.dev/web/compatible-website-design).
 * - LastPass: `data-lpignore="true"`
 *   (LastPass: "add 'data-lpignore'=true attribute to your form fields").
 * - Bitwarden: `data-bwignore` ("Web Developer Autofill Exclusion",
 *   https://community.bitwarden.com/t/where-is-the-developer-information/31547).
 * - Dashlane: `data-form-type="other"`
 *   (https://support.dashlane.com/hc/en-us/articles/4420122792594).
 */
export const NO_AUTOFILL = {
  autocomplete: 'off',
  'data-1p-ignore': true,
  'data-lpignore': 'true',
  'data-bwignore': 'true',
  'data-form-type': 'other',
} as const

/**
 * What a field holds, for `TextInput`'s `masked` and `search`:
 *
 * - `password`: an account password (sign-in, setup, change password), the only kind
 *   the browser and its password manager may treat as a password.
 * - `secret`: any other secret (an API key, a token, a credential's value).
 * - `search`: a filter or search box.
 */
export type Masked = 'password' | 'secret'

/** The `type` of the input: only a hidden account password is `password`, so nothing else is offered a saved login. */
export function inputType(masked: Masked | undefined, search: boolean | undefined, revealed: boolean): 'password' | 'search' | 'text' {
  if (masked === 'password' && !revealed)
    return 'password'
  return search ? 'search' : 'text'
}

/**
 * A secret is a text field drawn as dots with `-webkit-text-security`, which
 * MDN's browser-compat-data lists for Chrome 1, Safari 3 and Firefox 114, so
 * every current browser masks it. It is not in a standard, so it stays here.
 */
export function drawnAsDots(masked: Masked | undefined, revealed: boolean): boolean {
  return masked === 'secret' && !revealed
}

/** The attributes that keep browsers and password managers from filling or saving the field; none for an account password or a plain field. */
export function autofillAttrs(masked: Masked | undefined, search: boolean | undefined): typeof NO_AUTOFILL | Record<string, never> {
  return masked === 'secret' || search ? NO_AUTOFILL : {}
}

/**
 * The attributes of the hidden user name field that a form asking only for a
 * password (change password, add a passkey, confirm the password) carries.
 *
 * The field holds the account's email as `autocomplete="username"`, so the
 * password manager knows whose password the field holds, fills the right one and
 * updates that login on a change. Chromium's guidance for password forms
 * recommends it:
 * https://www.chromium.org/developers/design-documents/create-amazing-password-forms/
 * ("include a hidden username field in a change-password form"), as does
 * https://web.dev/articles/sign-in-form-best-practices#new-password.
 *
 * It is hidden from:
 *
 * - the eye (a one-pixel box, transparent),
 * - the keyboard (`tabindex="-1"`, read-only), and
 * - assistive technology (`aria-hidden`).
 *
 * It is not `display: none` or `type="hidden"`, which Chromium does not take for a
 * user name. Render it as `<input v-bind="hiddenUsername(email)">` before the
 * password field.
 */
export function hiddenUsername(email: string) {
  return {
    type: 'text',
    name: 'username',
    autocomplete: 'username',
    value: email,
    readonly: true,
    tabindex: -1,
    'aria-hidden': true,
    style: { position: 'absolute', width: '1px', height: '1px', overflow: 'hidden', opacity: 0, pointerEvents: 'none' },
  } as const
}
