import { createApp, shallowRef, watch } from 'vue'
import { createPageI18n, setPageLanguage } from '@demicodes/web-ui/i18n/text'
import { resolveLanguage } from '@demicodes/web-ui/i18n/languages'
import { applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import { applyTranscriptTextSize, DEFAULT_TRANSCRIPT_TEXT_SIZE } from '@demicodes/web-ui/theme/productAppearance'
import { installPhonePage } from '@demicodes/web-ui/ui/phone-page'
import { historySettled, setHistoryTraversal } from '@demicodes/web-ui/overlay/back-close'
import { reportPageErrorsWith } from '@demicodes/web-ui/infra/errors'
import { simulatedRequestId } from './fixtures/request-failures'
import { galleryState, persistGalleryState } from './gallery-state'
import { router } from './router'
import { installSizeAudit } from './size-audit'
import App from './App.vue'
import './style.css'

// The gallery speaks the language its chrome's switch, or Settings ›
// General's specimen, chose, as the product speaks the user's.
// System follows the browser's languages when they change, as the product's does.
const browserLanguages = shallowRef(navigator.languages)
const followBrowser = () => {
  browserLanguages.value = navigator.languages
}
window.addEventListener('languagechange', followBrowser)
const i18n = createPageI18n(resolveLanguage(galleryState.language, browserLanguages.value))
const stopLanguage = watch(
  () => resolveLanguage(galleryState.language, browserLanguages.value),
  setPageLanguage,
  { immediate: true },
)
applyThemeToDocument()
persistGalleryState()
// The transcript reads the user's text size, as in the product; Settings'
// specimen changes it from there.
applyTranscriptTextSize(DEFAULT_TRANSCRIPT_TEXT_SIZE)
// As in the product: a phone's page stays put, and a navigation waits for a closed sheet's history entry.
installPhonePage()
router.beforeEach(() => historySettled())
setHistoryTraversal((delta) => router.options.history.go(delta, false))
// The page's own failures go to the backend's log in the product, which
// answers with the report's request ID; the gallery has no backend, so its
// reporter answers with a simulated ID a moment later.
reportPageErrorsWith((report) => new Promise((resolve) => {
  setTimeout(() => resolve(simulatedRequestId(`${report.title}\n${report.detail}`)), 400)
}))
createApp(App).use(router).use(i18n).mount('#app')
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    window.removeEventListener('languagechange', followBrowser)
    stopLanguage()
  })
}
installSizeAudit((run) => router.afterEach(run))
