// The page each of the site's demos shows in a frame: Demi's own app frame,
// from `web-ui`, over a scripted conversation (`scenes.ts`). What a Host
// would answer, the work panel's browser, files and changes, comes from the
// gallery's simulated Host (`web-gallery/src/fixtures`), as the gallery's
// own specimens have it. The site's page sizes the frame like a desktop
// browser's window or a phone's screen, and the app lays itself out for that
// size as it does in the product.

import { createApp } from 'vue'
import { createPageI18n, setPageLanguage } from '@demicodes/web-ui/i18n/text'
import { applyProductAppearance, applyTranscriptTextSize, DEFAULT_TRANSCRIPT_TEXT_SIZE } from '@demicodes/web-ui/theme/productAppearance'
import { setTheme, applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import { installPhonePage } from '@demicodes/web-ui/ui/phone-page'
import { loadRegionRenderers } from '@demicodes/web-ui/infra/region-load'
import DemoApp from './DemoApp.vue'
import './style.css'

// A phone keeps its status bar and home indicator clear, as an iPhone 16's
// screen does; the page's frame cannot report them, so the demo names them.
if (new URLSearchParams(location.search).get('form') === 'phone') {
  document.documentElement.style.setProperty('--safe-top', '59px')
  document.documentElement.style.setProperty('--safe-bottom', '34px')
}

const i18n = createPageI18n('en')
setPageLanguage('en')
setTheme('light')
applyThemeToDocument()
applyProductAppearance({ tone: 'warm', accent: 'orange' })
applyTranscriptTextSize(DEFAULT_TRANSCRIPT_TEXT_SIZE)
createApp(DemoApp).use(i18n).mount('#app')
installPhonePage()
loadRegionRenderers()
