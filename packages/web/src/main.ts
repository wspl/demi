// The web application's composition root: the app, the router and the
// account-scoped stores. This package supplies the data, state, services and
// handlers of the reusable UI in `web-ui` and of each plugin's page; it never
// holds the UI itself.
//
// - Product state follows the synchronization channel, chat uses the
//   conversation client, each over a lane of the page connection, and
//   everything else is same-origin REST.
// - It imports no package by a computed name (the plugin page registry is a
//   static list `xtask contracts` writes), no `web-gallery`, and no Node
//   module outside its contract suite, which starts the backend, runner and
//   scripted machine manager processes.

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
import { followThemeColor } from '@demicodes/web-ui/theme/themeColor'
import { installPhonePage } from '@demicodes/web-ui/ui/phone-page'
import { createPageI18n, setPageLanguage } from '@demicodes/web-ui/i18n/text'
import { keepShownLanguage, lastShownLanguage, resolveLanguage } from '@demicodes/web-ui/i18n/languages'
import { errorFields, reportPageErrorsWith, showUnexpectedErrors } from '@demicodes/web-ui/infra/errors'
import { reportUncaughtErrors } from '@demicodes/web-ui/infra/uncaught'
import { logPageError, logPageWith, pageLog, pageLogSignedIn } from '@demicodes/web-ui/infra/log'
import { historySettled, setHistoryTraversal } from '@demicodes/web-ui/overlay/back-close'
import { useConversations } from './conversation/store'
import { closeDraftStorage } from './conversation/drafts'
import { useResources } from './state/resources'
import { useProduct } from './state/product'
import { followDirect } from './direct'
import { startServiceWorker } from './offline/page'
import { startAccountCache } from './state/offline'
import { accountTaken, forgetAccount, takeAccount } from './state/account-data'
import { onOwnerChange } from './state/account-owner'
import { usePreferences } from './state/preferences'
import { useNotifications } from './state/notifications'
import { useWorkPanel } from './conversation/work'
import { apiRequest, apiUrl, onSessionExpired } from './api/client'
import { configurePageConnection } from '@demicodes/web-ui/transport/page-connection'
import { sendPageLog } from './api/page-log'
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
// user's choice arrives.
const browserLanguages = shallowRef(navigator.languages)
const startLanguage = lastShownLanguage() ?? resolveLanguage('system', browserLanguages.value)
const i18n = createPageI18n(startLanguage)
setPageLanguage(startLanguage)
// The page connection, which carries every lane of the page's
// (`web-ui/src/transport/page-connection.ts`). The web browser does not say
// why its upgrade failed: an ended session answers it with 401, which a
// request learns and ends on the page as any 401 does.
configurePageConnection({
  url: () => apiUrl('/connect'),
  failedToOpen: () => void apiRequest('/auth/me', { waits: false }).catch(() => {}),
})
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
// The work panel's state is the page's address while it shows a conversation;
// it follows the router from the first navigation on.
const stopPanelAddress = useWorkPanel(pinia).attach(router)
const conversations = useConversations(pinia)
const session = useSession(pinia)
const product = useProduct(pinia)
const startup = new AbortController()
// The page's lines go to the backend's log; a failure the page finds itself is
// an error line there, and the error then names the request that carried it
// (`crates/backend-http/src/diagnostics.rs`).
const stopLog = logPageWith(sendPageLog)
// A page that is not signed in sends no lines, which the backend would refuse,
// and keeps them until it is.
const stopLogSession = watch(() => session.signedIn, pageLogSignedIn, { immediate: true })
const stopReports = reportPageErrorsWith(logPageError)
// The synchronization channel starts beside the session check; a page the check
// finds signed out has no channel.
product.start()
// The page's service worker answers the raw file requests over the page's
// direct channels and keeps the page for opening offline, for its lifetime
// (`packages/web/src/direct/raw-bridge.ts`).
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
// The browser holds one account's data: a sign-in makes it the account's,
// forgetting another's first, an account still to choose its password
// included, and a session that ended forgets it.
const stopTaking = watch(
  () => {
    const { current } = session
    return current.status === 'signedIn' || current.status === 'choosingPassword' ? current.user.id : undefined
  },
  (id) => {
    if (id) {
      void takeAccount(id).catch((error: unknown) => pageLog.warn('account_data.take_failed', errorFields(error)))
    }
  },
  { immediate: true },
)
const stopForgetting = onSessionExpired((over) => {
  const user = session.user
  // A sign-out forgets the account itself, once the backend took it.
  if (over && user && !session.leaving) {
    void forgetAccount(user.id).catch((error: unknown) => pageLog.warn('account_data.forget_failed', errorFields(error)))
  }
})
// Another tab that signed out, found its session ended or signed another
// account in has ended this page's account in the browser too, whose cookie
// the tabs share: the page loads itself again at the sign-in page, as its own
// sign-out does, which shows the product to an account that is signed in.
// It waits for no backend, so a page offline leaves as well.
const stopOwner = onOwnerChange((owner) => {
  if (session.user && owner !== session.user.id) {
    window.location.replace('/login')
  }
})
// The account's cache for opening offline: shown at once for the account the
// session names, and signed in as when the backend cannot be reached.
const accountCache = startAccountCache(session, product, { taken: accountTaken })
const restored = session.restore(startup.signal, accountCache.signInOffline).then(stopUnlessSignedIn).catch((error) => {
  // Hot replacement can dispose this composition root before startup finishes.
  if (!startup.signal.aborted) {
    throw error
  }
})
const preferences = usePreferences(pinia)
// Whether the page shows the defects it logs is the user's choice (Settings ›
// General); until the preferences arrive it does not.
const stopShowingErrors = watch(() => preferences.showUnexpectedErrors, showUnexpectedErrors, { immediate: true })
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
  // A session that ends because the account signs out is not one that expired.
  if (!session.signedIn || session.leaving) {
    return
  }
  conversations.saveDrafts()
  void settleSession()
})
// A page waiting on Choose Your Password goes on by itself once another page
// of the account chose it: its channel says so.
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
    // A session still alive is the browser's account again: what its end
    // forgot is written anew.
    void takeAccount(session.current.user.id).catch((error: unknown) => pageLog.warn('account_data.take_failed', errorFields(error)))
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
  // the backend.
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
const stopThemeColor = followThemeColor()
// The page speaks the language the user chose, or with System the first of
// the browser's that Demi speaks; a change of either, the choice made on
// another page included, applies at once.
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
// backend's canonical spelling does.
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
// as a mail client does while it sends attachments; the prompt is held only
// while such a message waits.
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
// What no code caught, from here on, is logged like any defect the page finds.
const stopUncaught = reportUncaughtErrors(app)
app.mount('#app')
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    startup.abort()
    stopUncaught()
    stopShowingErrors()
    stopReports()
    stopLog()
    stopLogSession()
    stopIdentity()
    accountCache.stop()
    stopTaking()
    stopForgetting()
    stopOwner()
    stopRetry()
    stopExpiry()
    stopChoice()
    stopAppearance()
    stopTheme()
    stopThemeColor()
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
    stopPanelAddress()
    app.unmount()
    disposePinia(pinia)
  })
}
