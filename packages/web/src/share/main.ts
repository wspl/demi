import { createApp, h } from 'vue'
import { applyProductAppearance, applyTranscriptTextSize, DEFAULT_ACCENT, DEFAULT_TRANSCRIPT_TEXT_SIZE, productAppearance } from '@demicodes/web-ui/theme/productAppearance'
import { followSystemTheme } from '@demicodes/web-ui/theme/appTheme'
import { createPageI18n, setPageLanguage } from '@demicodes/web-ui/i18n/text'
import { resolveLanguage } from '@demicodes/web-ui/i18n/languages'
import SharedLinkMissing from '@demicodes/web-ui/share/SharedLinkMissing.vue'
import SharePage from './SharePage.vue'
import { shareToken } from './address'
import '../style.css'

/*
 * The shared page's composition root (`sharing.md` § Safety): its own small
 * entry of the build, which loads none of the product's and calls only the
 * share's public routes. It has no account, so it speaks the reader's
 * system's language and follows its system's light or dark mode, and keeps
 * neither, so the product's choices on the same site stay as they were.
 */
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

const token = shareToken(location.pathname)
createApp({ render: () => (token === null ? h(SharedLinkMissing) : h(SharePage, { token })) })
  .use(i18n)
  .mount('#app')
