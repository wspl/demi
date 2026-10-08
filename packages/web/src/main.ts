import { createApp, watch } from 'vue'
import { createPinia, disposePinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import {
  applyProductAppearance,
  applyTranscriptTextSize,
  productAppearance,
} from '@demicodes/web-ui/theme/productAppearance'
import {
  appThemeStore,
  applyThemeToDocument,
  setThemeChoice,
} from '@demicodes/web-ui/theme/appTheme'
import { useConversations } from './conversation/store'
import { closeDraftStorage } from './conversation/drafts'
import { useResources } from './state/resources'
import { useProduct } from './state/product'
import { followDirect } from './direct'
import { startRawBridge } from './direct/raw-bridge'
import { usePreferences } from './state/preferences'
import { useNotifications } from './state/notifications'
import { onSessionExpired } from './api/client'
import { useSession } from './auth/session'
import ChatPage from './conversation/ChatPage.vue'
import LoginPage from './auth/LoginPage.vue'
import SetupPage from './auth/SetupPage.vue'
import App from './App.vue'
import { SETTINGS_ROUTE } from './settings/address'
import { returnAddress, signInAddress } from './auth/return'
import './style.css'

const pinia = createPinia()
const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      redirect: '/chat',
    },
    // Signed in, App shows the chat of the address, or of the page settings
    // opened over (`settings/address.ts`); the records name it for the router.
    {
      path: '/chat/:id?',
      component: ChatPage,
    },
    {
      path: '/settings/:section?/:detail?',
      name: SETTINGS_ROUTE,
      component: ChatPage,
    },
    {
      path: '/login',
      component: LoginPage,
    },
    {
      path: '/setup',
      component: SetupPage,
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
const product = useProduct(pinia)
const startup = new AbortController()
// The synchronization channel starts beside the session check
// (`web-application.md` § Requests for one action); a page the check finds
// signed out has no channel.
product.start()
// The page answers its service worker's raw file requests over its direct
// channels for its lifetime (`direct-channel.md` § Bytes the browser
// fetches itself).
startRawBridge()
// The page makes the peer of the device a conversation it shows runs on, and
// follows what changes its paths, for its lifetime.
followDirect()
/** A page signed out, or of a Demi without accounts, follows no state; one whose check failed keeps the channel for its Retry. */
function stopUnlessSignedIn(): void {
  const { status } = session.current
  if (status === 'signedOut' || status === 'setupNeeded') {
    product.stop()
  }
}
const restored = session.restore(startup.signal).then(stopUnlessSignedIn).catch((error) => {
  // Hot replacement can dispose this composition root before startup finishes.
  if (!startup.signal.aborted) {
    throw error
  }
})
const preferences = usePreferences(pinia)
const notifications = useNotifications(pinia)
const stopIdentity = watch(
  () => session.user?.id,
  (id, previous) => {
    if (previous && previous !== id) {
      notifications.stop()
      conversations.stopAll()
      preferences.stop()
      product.stop()
    }
    if (id) {
      void conversations.initialize()
      // A click on a notification opens its conversation.
      notifications.start((conversationId) => void router.push(`/chat/${conversationId}`))
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
  void router.replace(signInAddress(router.currentRoute.value.fullPath, 'expired'))
})
// A first check the backend failed shows its failure, whose Retry checks
// again: what the retry finds decides the page's pages as the first check would.
const stopRetry = watch(
  () => session.current.status,
  (status, previous) => {
    if (previous !== 'failed' || status === 'failed') {
      return
    }
    stopUnlessSignedIn()
    const { path, query, hash } = router.currentRoute.value
    void router.replace({ path, query, hash, force: true })
  },
)
router.beforeEach(async (to) => {
  await restored
  if (startup.signal.aborted) {
    return false
  }
  // Each session state has its pages: setup while no account exists, sign-in
  // without a session, and the product with one.
  const status = session.current.status
  if (status === 'failed') {
    // The address stays for the retry; the page shows the failure over it.
    return true
  }
  if (status === 'setupNeeded') {
    return to.path === '/setup' ? true : '/setup'
  }
  if (status === 'signedIn') {
    // Signing in goes on to the page that was opened signed out.
    return to.path === '/login' || to.path === '/setup' ? returnAddress(to.query) : true
  }
  return to.path === '/login' ? true : signInAddress(to.fullPath)
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
const stopLocale = watch(
  [() => product.snapshot?.preferences, () => appThemeStore.state.mode],
  () => void preferences.reportBrowser(),
  { immediate: true },
)
const reportBrowser = () => void preferences.reportBrowser()
window.addEventListener('languagechange', reportBrowser)
const saveDrafts = () => {
  // The backend's saves go first: the web browser sends them after the page is gone.
  conversations.flushDrafts(true)
  conversations.keepLastWords()
  conversations.saveDrafts()
  closeDraftStorage()
}
window.addEventListener('pagehide', saveDrafts)
const refreshVisible = () => {
  if (document.visibilityState === 'hidden') {
    // A hidden page saves what its user typed, not waiting for a pause.
    conversations.flushDrafts()
    return
  }
  if (document.visibilityState !== 'visible' || !session.signedIn) {
    return
  }
  void preferences.reportBrowser()
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
    stopRetry()
    stopExpiry()
    stopAppearance()
    stopTheme()
    stopLocale()
    notifications.stop()
    window.removeEventListener('languagechange', reportBrowser)
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
