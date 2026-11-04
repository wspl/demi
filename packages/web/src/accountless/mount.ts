import { createApp, h, type Component } from 'vue'
import { applyProductAppearance, applyTranscriptTextSize, DEFAULT_ACCENT, DEFAULT_TRANSCRIPT_TEXT_SIZE, productAppearance } from '@demicodes/web-ui/theme/productAppearance'
import { followSystemTheme } from '@demicodes/web-ui/theme/appTheme'
import { createPageI18n, setPageLanguage } from '@demicodes/web-ui/i18n/text'
import { resolveLanguage } from '@demicodes/web-ui/i18n/languages'

/**
 * The token a link page's address names: `/<prefix>/<token>`, such as a
 * share's `/s/<token>`; null for any other address.
 */
function linkToken(prefix: string, pathname: string): string | null {
  const token = new RegExp(`^/${prefix}/([^/]+)/?$`).exec(pathname)?.[1]
  return token === undefined ? null : decodeURIComponent(token)
}

/**
 * Mounts the page of a link that opens without an account, a share's or a
 * view link's, as its build entry's composition root: `page` for the token
 * its address under `prefix` names, `missing` for an address that names
 * none. It speaks the reader's system's language and follows its system's
 * light or dark mode, and keeps neither, so the product's choices on the
 * same site stay as they were.
 */
export function mountLinkPage(prefix: string, missing: Component, page: Component<{ token: string }>): void {
  const language = () => resolveLanguage('system', navigator.languages)
  const i18n = createPageI18n(language())
  setPageLanguage(language())
  window.addEventListener('languagechange', () => setPageLanguage(language()))
  for (const [axis, value] of Object.entries(productAppearance)) {
    document.documentElement.setAttribute(`data-${axis}`, value)
  }
  applyProductAppearance({ tone: productAppearance.tone, accent: DEFAULT_ACCENT })
  applyTranscriptTextSize(DEFAULT_TRANSCRIPT_TEXT_SIZE)
  followSystemTheme()
  const token = linkToken(prefix, location.pathname)
  createApp({ render: () => (token === null ? h(missing) : h(page, { token })) })
    .use(i18n)
    .mount('#app')
}
