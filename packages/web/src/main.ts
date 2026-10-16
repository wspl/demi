import { createApp, shallowRef, watch } from 'vue'
import { until } from '@vueuse/core'
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
import { installPhonePage } from '@demicodes/web-ui/ui/phone-page'
import { createPageI18n, setPageLanguage } from '@demicodes/web-ui/i18n/text'
import { keepShownLanguage, lastShownLanguage, resolveLanguage } from '@demicodes/web-ui/i18n/languages'
import { reportPageErrorsWith } from '@demicodes/web-ui/infra/errors'
import { historySettled, setHistoryTraversal } from '@demicodes/web-ui/overlay/back-close'
import { useConversations } from './conversation/store'
import { closeDraftStorage } from './conversation/drafts'
import { useResources } from './state/resources'
import { useProduct } from './state/product'
import { followDirect } from './direct'
import { startServiceWorker } from './offline/page'
import { startAccountCache } from './state/offline'
import { usePreferences } from './state/preferences'
import { useNotifications } from './state/notifications'
import { onSessionExpired } from './api/client'
import { sendPageErrorReport } from './api/page-errors'
import { useSession } from './auth/session'
import ChatPage from './conversation/ChatPage.vue'
import LoginPage from './auth/LoginPage.vue'
import SetupPage from './auth/SetupPage.vue'
import HandoffPage from './auth/HandoffPage.vue'
import App from './App.vue'
import { SETTINGS_ROUTE } from './settings/address'
import { returnAddress, signInAddress } from './auth/return'
import './style.css'

// The page's text instance, before anything reads a text; it speaks the
// language it last showed, or the browser's the first time, until the
// user's choice arrives (`localization.md` § Choosing the language).
const browserLanguages = shallowRef(navigator.languages)
const startLanguage = lastShownLanguage() ?? resolveLanguage('system', browserLanguages.value)
const i18n = createPageI18n(startLanguage)
setPageLanguage(startLanguage)
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
    // A phone's camera opens a handoff code here, signed in or not.
    {
      path: '/handoff',
      component: HandoffPage,
    },
    {
      path: '/:pathMatch(.*)*',
      redirect: '/chat',
    },
  ],
})
// A navigation waits while a closed phone sheet takes its entry off the history (`back-close.ts`).
router.beforeEach(() => historySettled())
setHistoryTraversal((delta) => router.options.history.go(delta, false))
const resources = useResources(pinia)
const conversations = useConversations(pinia)
const session = useSession(pinia)
const product = useProduct(pinia)
const startup = new AbortController()
// A failure the page finds itself goes to the backend's log, which the error
// then names (`backend.md` § Request IDs and the server log).
const stopReports = reportPageErrorsWith(sendPageErrorReport)
// The synchronization channel starts beside the session check
// (`web-application.md` § Requests for one action); a page the check finds
// signed out has no channel.
product.start()
// The page's service worker answers the raw file requests over the page's
// direct channels and keeps the page for opening offline, for its lifetime
// (`direct-channel.md` § Bytes the browser fetches itself).
startServiceWorker()
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
// The account's cache for opening offline (`web-application.md` § Installing
// and opening offline): shown at once for the account the session names,
// and signed in as when the backend cannot be reached.
const accountCache = startAccountCache(session, product)
const restored = session.restore(startup.signal, accountCache.signInOffline).then(stopUnlessSignedIn).catch((error) => {
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
  void settleSession()
})
// A page waiting on Choose Your Password goes on by itself once another page
// of the account chose it: its channel says so (`web-api-settings.md` § Account API).
const stopChoice = watch(
  () => product.account?.choosesPassword,
  (choosing) => {
    const { current } = session
    if (choosing === false && current.status === 'choosingPassword' && current.user.choosesPassword) {
      session.finishChoosing()
    }
  },
)
/**
 * A session the page's requests or channel found ended: the identity says
 * whether it ended (signed out) or its account must choose a password first
 * (the wizard asks); a session still alive follows the state again.
 */
async function settleSession(): Promise<void> {
  try {
    await session.restore()
  } catch {
    session.current = { status: 'signedOut', reason: 'expired' }
  }
  const { status } = session.current
  if (status === 'signedOut') {
    void router.replace(signInAddress(router.currentRoute.value.fullPath, 'expired'))
  } else if (status === 'signedIn') {
    product.reconnect()
  }
}
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
  // The session has a state once its check answers, or once the page signs
  // in as the account it kept for opening offline while the check waits for
  // the backend (`web-application.md` § Installing and opening offline).
  await Promise.race([restored, until(() => session.current.status).not.toBe('checking')])
  if (startup.signal.aborted) {
    return false
  }
  // Each session state has its pages: setup while no account exists, sign-in
  // without a session, and the product with one.
  const status = session.current.status
  // The handoff page signs the phone in, in whatever state it was.
  if (to.path === '/handoff') {
    return true
  }
  if (status === 'failed') {
    // The address stays for the retry; the page shows the failure over it.
    return true
  }
  if (status === 'setupNeeded' || status === 'passkeyStep') {
    return to.path === '/setup' ? true : '/setup'
  }
  if (status === 'signedIn' || status === 'choosingPassword') {
    // Signing in goes on to the page that was opened signed out.
    return to.path === '/login' || to.path === '/setup' ? returnAddress(to.query) : true
  }
  return to.path === '/login' ? true : signInAddress(to.fullPath)
})
for (const [axis, value] of Object.entries(productAppearance)) {
  document.documentElement.setAttribute(`data-${axis}`, value)
}
// Each value is put on the document when it changes, not each time the
// product's state does: the appearance is made anew for every message of
// the sync channel, and an attribute written again, even unchanged, tells
// what watches the document's appearance to draw again.
const stopAppearance = watch(
  [
    () => resources.appearance.tone,
    () => resources.appearance.accent,
    () => resources.appearance.fontSize,
    () => resources.appearance.theme,
  ],
  ([tone, accent, fontSize, theme]) => {
    applyProductAppearance({ tone, accent })
    applyTranscriptTextSize(fontSize)
    setThemeChoice(theme)
  },
  { immediate: true },
)
const stopTheme = applyThemeToDocument()
// The page speaks the language the user chose, or with System the first of
// the browser's that Demi speaks; a change of either, the choice made on
// another page included, applies at once (`localization.md` § Choosing the
// language).
const stopLanguage = watch(
  () => (product.snapshot ? resolveLanguage(preferences.language, browserLanguages.value) : null),
  (language) => {
    // Until the preferences arrive the page stays in the language it
    // started in.
    if (language !== null) {
      setPageLanguage(language)
      keepShownLanguage(language)
    }
  },
  { immediate: true },
)
// The page reports its own facts when they may have changed: once the
// stored preferences first arrive, when its theme changes, and on the events
// below; never because the stored ones changed, which another page or the
// backend's canonical spelling does (`web-api-settings.md` § User preferences).
const stopLocale = watch(
  [() => product.snapshot?.preferences !== undefined, () => appThemeStore.state.mode],
  () => void preferences.reportBrowser(),
  { immediate: true },
)
const reportBrowser = () => {
  browserLanguages.value = navigator.languages
  void preferences.reportBrowser()
}
window.addEventListener('languagechange', reportBrowser)
const saveDrafts = () => {
  // The backend's saves go first: the web browser sends them after the page is gone.
  void preferences.flush()
  conversations.flushDrafts(true)
  conversations.keepLastWords()
  conversations.saveDrafts()
  void conversations.keepCopies()
  closeDraftStorage()
}
window.addEventListener('pagehide', saveDrafts)
// A page closed while a sent message waits for its files' uploads asks first,
// as a mail client does while it sends attachments (`composer.md` § Writing a
// message); the prompt is held only while such a message waits.
const askBeforeLeaving = (event: BeforeUnloadEvent) => event.preventDefault()
const stopLeaving = watch(
  () => conversations.waitingForUploads,
  (waiting) => {
    if (waiting) {
      window.addEventListener('beforeunload', askBeforeLeaving)
    } else {
      window.removeEventListener('beforeunload', askBeforeLeaving)
    }
  },
)
const refreshVisible = () => {
  if (document.visibilityState === 'hidden') {
    // A hidden page saves what its user typed and chose, not waiting for a
    // pause.
    void preferences.flush()
    conversations.flushDrafts()
    void conversations.keepCopies()
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
const stopPhonePage = installPhonePage()
const app = createApp(App).use(pinia).use(router).use(i18n)
app.mount('#app')
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    startup.abort()
    stopReports()
    stopIdentity()
    accountCache.stop()
    stopRetry()
    stopExpiry()
    stopChoice()
    stopAppearance()
    stopTheme()
    stopLanguage()
    stopLocale()
    notifications.stop()
    window.removeEventListener('languagechange', reportBrowser)
    conversations.stopAll()
    closeDraftStorage()
    preferences.stop()
    product.stop()
    window.removeEventListener('pagehide', saveDrafts)
    stopLeaving()
    window.removeEventListener('beforeunload', askBeforeLeaving)
    window.removeEventListener('focus', refreshVisible)
    document.removeEventListener('visibilitychange', refreshVisible)
    stopPhonePage()
    app.unmount()
    disposePinia(pinia)
  })
}
