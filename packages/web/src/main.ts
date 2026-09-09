import { createApp, watch } from 'vue'
import { createPinia, disposePinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import {
  applyProductAppearance,
  applyTranscriptTextSize,
  productAppearance,
} from '@demicodes/web-ui/theme/productAppearance'
import {
  applyThemeToDocument,
  setThemeChoice,
} from '@demicodes/web-ui/theme/appTheme'
import { useConversations } from './conversation/store'
import { closeDraftStorage } from './conversation/drafts'
import { useResources } from './state/resources'
import { useProduct } from './state/product'
import { usePreferences } from './state/preferences'
import { onSessionExpired } from './api/client'
import { useSession } from './auth/session'
import ChatPage from './conversation/ChatPage.vue'
import LoginPage from './auth/LoginPage.vue'
import App from './App.vue'
import './style.css'

const pinia = createPinia()
const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      redirect: '/chat',
    },
    {
      path: '/chat/:id?',
      component: ChatPage,
    },
    {
      path: '/login',
      component: LoginPage,
    },
    {
      path: '/:pathMatch(.*)*',
      redirect: '/chat',
    },
  ],
})
const resources = useResources(pinia)
const conversations = useConversations(pinia)
const session = useSession(pinia)
const startup = new AbortController()
const restored = session.restore(startup.signal).catch((error) => {
  // Hot replacement can dispose this composition root before startup finishes.
  if (!startup.signal.aborted) {
    throw error
  }
})
const product = useProduct(pinia)
const preferences = usePreferences(pinia)
const stopIdentity = watch(
  () => session.user?.id,
  (id, previous) => {
    if (previous && previous !== id) {
      conversations.stopAll()
      preferences.stop()
      product.stop()
    }
    if (id) {
      void conversations.initialize()
    }
  },
  { immediate: true },
)
const stopExpiry = onSessionExpired(() => {
  if (!session.signedIn) {
    return
  }
  conversations.saveDrafts()
  session.current = {
    status: 'signedOut',
    reason: 'expired',
  }
  void router.replace('/login?reason=expired')
})
router.beforeEach(async (to) => {
  await restored
  if (startup.signal.aborted) {
    return false
  }
  if (!session.signedIn && to.path !== '/login') {
    return '/login'
  }
  if (session.signedIn && to.path === '/login') {
    return '/chat'
  }
  return true
})
for (const [axis, value] of Object.entries(productAppearance)) {
  document.documentElement.setAttribute(`data-${axis}`, value)
}
const stopAppearance = watch(
  () => resources.appearance,
  (appearance) => {
    applyProductAppearance(appearance)
    applyTranscriptTextSize(appearance.fontSize)
    setThemeChoice(appearance.theme)
  },
  {
    immediate: true,
    deep: true,
  },
)
const stopTheme = applyThemeToDocument()
const saveDrafts = () => {
  conversations.saveDrafts()
  closeDraftStorage()
}
window.addEventListener('pagehide', saveDrafts)
const refreshVisible = () => {
  if (document.visibilityState !== 'visible' || !session.signedIn) {
    return
  }
  void product.revalidate()
  const id = product.activeConversationId
  if (id) {
    void conversations.markRead(id)
  }
}
window.addEventListener('focus', refreshVisible)
document.addEventListener('visibilitychange', refreshVisible)
const app = createApp(App).use(pinia).use(router)
app.mount('#app')
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    startup.abort()
    stopIdentity()
    stopExpiry()
    stopAppearance()
    stopTheme()
    conversations.stopAll()
    closeDraftStorage()
    preferences.stop()
    product.stop()
    window.removeEventListener('pagehide', saveDrafts)
    window.removeEventListener('focus', refreshVisible)
    document.removeEventListener('visibilitychange', refreshVisible)
    app.unmount()
    disposePinia(pinia)
  })
}
