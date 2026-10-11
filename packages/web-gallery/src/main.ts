import { createApp } from 'vue'
import { applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import { applyTranscriptTextSize, DEFAULT_TRANSCRIPT_TEXT_SIZE } from '@demicodes/web-ui/theme/productAppearance'
import { installPhonePage } from '@demicodes/web-ui/ui/phone-page'
import { historySettled } from '@demicodes/web-ui/overlay/back-close'
import { persistGalleryState } from './gallery-state'
import { router } from './router'
import { installClipAudit } from './clip-audit'
import { installSizeAudit } from './size-audit'
import App from './App.vue'
import './style.css'

applyThemeToDocument()
persistGalleryState()
// The transcript reads the user's text size, as in the product; Settings'
// specimen changes it from there.
applyTranscriptTextSize(DEFAULT_TRANSCRIPT_TEXT_SIZE)
// As in the product: a phone's page stays put, and a navigation waits for a closed sheet's history entry.
installPhonePage()
router.beforeEach(() => historySettled())
createApp(App).use(router).mount('#app')
installClipAudit((run) => router.afterEach(run))
installSizeAudit((run) => router.afterEach(run))
