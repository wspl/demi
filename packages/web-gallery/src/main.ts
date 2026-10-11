import { createApp } from 'vue'
import { applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import { applyTranscriptTextSize, DEFAULT_TRANSCRIPT_TEXT_SIZE } from '@demicodes/web-ui/theme/productAppearance'
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
createApp(App).use(router).mount('#app')
installClipAudit((run) => router.afterEach(run))
installSizeAudit((run) => router.afterEach(run))
