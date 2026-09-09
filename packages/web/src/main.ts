import { createApp, watch } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import {
  applyProductAppearance,
  applyTranscriptTextSize,
  productAppearance,
} from '@demicodes/web-ui/theme/productAppearance'
import {
  applyThemeToDocument,
  themeChoice,
} from '@demicodes/web-ui/theme/appTheme'
import { useConversations } from './conversation/store'
import { useResources } from './prototype/resources'
import { useSession } from './auth/session'
import { showToast } from '@demicodes/web-ui/infra/toast'
import ChatPage from './conversation/ChatPage.vue'
import LoginPage from './auth/LoginPage.vue'
import App from './App.vue'
import './style.css'

const pinia = createPinia()
const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/chat/welcome' },
    { path: '/chat/:id?', component: ChatPage },
    { path: '/login', component: LoginPage },
    { path: '/:pathMatch(.*)*', redirect: '/chat' },
  ],
})
const resources = useResources(pinia)
const conversations = useConversations(pinia)
const session = useSession(pinia)
const startup = new AbortController()
const restored = session.restore(startup.signal).catch((error) => {
  // Hot replacement can dispose this composition root before startup finishes.
  if (!startup.signal.aborted)
    throw error
})
const stopIdentity = watch(() => session.user, (user) => {
  resources.username = user?.nickname ?? ''
  resources.settings.account.email = user?.email ?? ''
}, { immediate: true })
router.beforeEach(
  async (to, from) => {
    await restored
    if (startup.signal.aborted)
      return false
    if (session.signedIn && from.matched.length && from.path !== '/login') {
      try {
        await session.restore(startup.signal)
      } catch (error) {
        if (startup.signal.aborted)
          return false
        showToast({
          title: 'Could not check your session',
          message: error instanceof Error ? error.message : String(error),
          tone: 'danger',
        })
        return false
      }
      if (!session.signedIn) {
        window.location.replace('/login?reason=expired')
        return false
      }
    }
    if (!session.signedIn && to.path !== '/login')
      return '/login'
    if (session.signedIn && to.path === '/login')
      return '/chat/welcome'
    return true
  }
)
for (const [axis, value] of Object.entries(productAppearance)) {
  document.documentElement.setAttribute(`data-${axis}`, value)
}
// Start the appearance from the locally saved General settings.
resources.settings.general.theme = themeChoice()
applyProductAppearance(resources.settings.general)
applyTranscriptTextSize(resources.settings.general.fontSize)
applyThemeToDocument()
const app = createApp(App).use(pinia).use(router)
void router.isReady().then(() => {
  if (!startup.signal.aborted)
    app.mount('#app')
})
const timer = window.setInterval(() => conversations.advance(), 80)
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    startup.abort()
    stopIdentity()
    clearInterval(timer)
    app.unmount()
  })
}
