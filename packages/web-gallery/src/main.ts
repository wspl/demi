import { createApp } from 'vue'
import { applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import { persistGalleryState } from './gallery-state'
import { router } from './router'
import { installClipAudit } from './clip-audit'
import { installSizeAudit } from './size-audit'
import App from './App.vue'
import './style.css'

applyThemeToDocument()
persistGalleryState()
createApp(App).use(router).mount('#app')
installClipAudit((run) => router.afterEach(run))
installSizeAudit((run) => router.afterEach(run))
