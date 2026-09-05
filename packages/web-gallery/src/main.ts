import { createApp } from 'vue'
import { applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import { persistGalleryState } from './gallery-state'
import { router } from './router'
import App from './App.vue'
import './style.css'

applyThemeToDocument()
persistGalleryState()
createApp(App).use(router).mount('#app')
