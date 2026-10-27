import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import { z } from 'zod'
import { errorFields, reportError } from '@demicodes/web-ui/infra/errors'
import { logStorageFailed, pageLog } from '@demicodes/web-ui/infra/log'
import { conversationTitle } from '@demicodes/web-ui/agent/conversation-title'
import { pageLanguage } from '@demicodes/web-ui/i18n/text'
import { requestTitleWords } from '@demicodes/web-ui/permissions/types'
import { FRAME_INPUT, USER_INPUT_EVENTS } from '@demicodes/web-ui/ui/user-input'
import { t } from './locales/text'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  pushKeySchema,
  pushSubscriptionKeysSchema,
  type PushEvents,
  type PushSubscriptionBody,
} from '../api/generated/web-api'
import { OPEN_CONVERSATION, WORDS_CACHE, WORDS_KEY, type NotificationWords } from '../offline/notifications'
import { useProduct } from './product'

/**
 * Notifications: push notifications, which the browser's push service brings to
 * the page's service worker whether or not a page is open. This browser's
 * settings live in its storage. While they are on and the browser allows them,
 * the page:
 *
 * - Keeps the browser subscribed for its session, with the events it chose
 *   and whether it stays quiet while the user is on another device.
 * - Tells the backend on its synchronization channel whether it is in front
 *   of the user, focused and visible, and which conversation it shows then:
 *   no turn's end is notified to this browser while it is, and nothing of
 *   that conversation.
 * - Leaves the service worker the words it says notifications in.
 *
 * Whatever the settings, the page tells the backend when the user acts on it
 * while it is visible, which keeps its session in use: the browsers of the
 * user's other sessions that stay quiet then are not notified of the end of
 * a turn.
 *
 * A click on a notification opens its conversation.
 *
 * A browser's subscription serves one session: signing out unsubscribes the
 * browser, and a session that finds it subscribed by another makes a new
 * subscription before it sends one, so nothing queued for the account signed
 * out before shows after another signs in.
 */

const settingsSchema = z.object({
  /** The user turned notifications on in this browser. */
  enabled: z.boolean(),
  turnFinishes: z.boolean(),
  turnFails: z.boolean(),
  needsPermission: z.boolean(),
  agentNotifies: z.boolean(),
  /** The end of a turn does not notify while the user is on another device; on for a setting stored before it. */
  quietWhileElsewhere: z.boolean().default(true),
})
export type NotificationSettings = z.infer<typeof settingsSchema>

/** Off until turned on; once on, everything notifies. */
const DEFAULT_SETTINGS: NotificationSettings = {
  enabled: false,
  turnFinishes: true,
  turnFails: true,
  needsPermission: true,
  agentNotifies: true,
  quietWhileElsewhere: true,
}

/** The setting belongs to the browser, not the account, so one key holds it for every user of the browser. */
const STORAGE_KEY = 'demi.notifications'

function readSettings(): NotificationSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? settingsSchema.parse(JSON.parse(raw)) : { ...DEFAULT_SETTINGS }
  } catch (error) {
    // The browser's storage is optional: without it, or with an entry that
    // does not read, notifications are off until turned on.
    logStorageFailed('notification_settings', 'read', error)
    return { ...DEFAULT_SETTINGS }
  }
}

function writeSettings(settings: NotificationSettings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings))
  } catch (error) {
    // Private browsing and storage quotas keep the setting for this page alone.
    logStorageFailed('notification_settings', 'write', error)
  }
}

/** Which session made the browser's subscription, which belongs to the browser too. */
const MAKER_KEY = 'demi.notifications.session'

function subscriptionMaker(): string | null {
  try {
    return localStorage.getItem(MAKER_KEY)
  } catch (error) {
    // Without storage, the page cannot tell: it makes a new subscription.
    logStorageFailed('subscribed_session', 'read', error)
    return null
  }
}

function rememberMaker(session: string | null): void {
  try {
    if (session === null) {
      localStorage.removeItem(MAKER_KEY)
    } else {
      localStorage.setItem(MAKER_KEY, session)
    }
  } catch (error) {
    // The next session then finds no maker and makes a subscription of its own.
    logStorageFailed('subscribed_session', 'write', error)
  }
}

/** What the browser allows this site; `unsupported` for one that cannot push. */
export type BrowserPermission = NotificationPermission | 'unsupported'

function browserPermission(): BrowserPermission {
  const canPush = typeof Notification !== 'undefined'
    && typeof PushManager !== 'undefined'
    && typeof navigator !== 'undefined'
    && 'serviceWorker' in navigator
  return canPush ? Notification.permission : 'unsupported'
}

/**
 * Safari on an iPhone or iPad outside the Home Screen, which pushes only to
 * Demi added there: its `navigator.standalone`, a property of Safari's
 * alone, is false; true in the app the Home Screen opens.
 */
export function needsHomeScreen(browser: object): boolean {
  return Reflect.get(browser, 'standalone') === false
}


/** The events `settings` chose. */
function eventsOf(settings: NotificationSettings): PushEvents {
  return {
    turnFinishes: settings.turnFinishes,
    turnFails: settings.turnFails,
    needsPermission: settings.needsPermission,
    agentNotifies: settings.agentNotifies,
  }
}

/** The words notifications are said in, in the page's language, for the service worker. */
function notificationWords(): NotificationWords {
  return {
    untitled: conversationTitle(null),
    turnStopped: t('notifications.turnStopped'),
    saveCredential: t('notifications.saveCredential', { name: '{name}' }),
    ...requestTitleWords(),
  }
}

/** Leaves the service worker the words notifications are said in. */
async function leaveWords(): Promise<void> {
  const cache = await caches.open(WORDS_CACHE)
  await cache.put(WORDS_KEY, new Response(JSON.stringify(notificationWords()), { headers: { 'content-type': 'application/json' } }))
}

/** A subscription's endpoint and keys, as the backend keeps them. */
function subscriptionBody(subscription: PushSubscription, events: PushEvents, quietWhileElsewhere: boolean): PushSubscriptionBody {
  return {
    endpoint: subscription.endpoint,
    keys: pushSubscriptionKeysSchema.parse(subscription.toJSON().keys),
    events,
    quietWhileElsewhere,
  }
}

/** A browser's subscription, as keeping it for a session needs it. */
export interface BrowserSubscription {
  readonly endpoint: string
  unsubscribe(): Promise<boolean>
}

/** What keeping the browser subscribed for a session takes. */
export interface Subscribing<S extends BrowserSubscription> {
  /** The browser's subscription, if it has one. */
  current(): Promise<S | null>
  /** Makes a new subscription of the browser. */
  make(): Promise<S>
  /** Sends it to the backend for the session. */
  put(subscription: S): Promise<void>
  /** Which session made the browser's subscription, as the page remembers it. */
  maker(): string | null
  remember(session: string): void
}

/**
 * Keeps the browser subscribed for `session`: a subscription another
 * session made, or one another live session holds, is replaced by a new
 * one before it is sent, so it serves this session alone.
 */
export async function subscribeFor<S extends BrowserSubscription>(session: string, browser: Subscribing<S>): Promise<S> {
  let subscription = await browser.current()
  if (subscription && browser.maker() !== session) {
    await subscription.unsubscribe()
    subscription = null
  }
  for (let fresh = false; ; fresh = true) {
    if (!subscription) {
      subscription = await browser.make()
      browser.remember(session)
    }
    try {
      await browser.put(subscription)
      return subscription
    } catch (error) {
      if (fresh || !(error instanceof ApiError && error.code === 'push_endpoint_taken')) {
        throw error
      }
      await subscription.unsubscribe()
      subscription = null
    }
  }
}

/**
 * Unsubscribes the browser as its session signs out: the backend forgets
 * the session's subscription, and the push service the browser's, so
 * nothing queued for the account reaches it. A failure is logged: signing
 * out goes on, and the session's end removes its subscription anyway.
 */
export async function unsubscribeOnSignOut(): Promise<void> {
  if (typeof navigator === 'undefined' || !('serviceWorker' in navigator) || typeof PushManager === 'undefined') {
    return
  }
  try {
    const registration = await navigator.serviceWorker.getRegistration()
    const subscription = await registration?.pushManager.getSubscription()
    rememberMaker(null)
    await apiRequest('/push/subscription', { method: 'DELETE', waits: false })
    await subscription?.unsubscribe()
  } catch (error) {
    pageLog.warn('push_subscription.removed_failed', errorFields(error))
  }
}

export const useNotifications = defineStore('notifications', () => {
  const product = useProduct()
  const settings = ref(readSettings())
  watch(settings, writeSettings, { deep: true })
  const permission = ref<BrowserPermission>(browserPermission())
  /** Safari outside the Home Screen: turning notifications on shows how to add Demi there. */
  const homeScreen = typeof navigator !== 'undefined' && needsHomeScreen(navigator)

  /** Asks the browser for permission to notify; a browser that blocks the site answers without asking. */
  async function requestPermission(): Promise<NotificationPermission> {
    if (permission.value === 'unsupported') {
      return 'denied'
    }
    const answer = await Notification.requestPermission()
    permission.value = answer
    return answer
  }

  /** Set from `start` to `stop`. */
  let lifetime: AbortController | null = null
  /** The browser's subscription as the backend keeps it, while it does. */
  let subscribed: PushSubscription | null = null
  /** The change of the subscription under way, which the next waits for. */
  let changing: Promise<void> = Promise.resolve()

  /** Whether the page is in front of the user: visible and focused. */
  function inFront(): boolean {
    return document.visibilityState === 'visible' && document.hasFocus()
  }

  /**
   * Keeps the browser subscribed with the events it chose while
   * notifications are on and allowed, and unsubscribed otherwise; one
   * change at a time, in order.
   */
  function follow(signal: AbortSignal, session: string | undefined): void {
    const wanted = settings.value.enabled && permission.value === 'granted'
    const events = eventsOf(settings.value)
    const quiet = settings.value.quietWhileElsewhere
    if (wanted && session === undefined) {
      // The session is known once the state arrives, which follows again.
      return
    }
    changing = changing.then(async () => {
      if (signal.aborted) {
        return
      }
      try {
        if (wanted && session !== undefined) {
          await subscribe(session, events, quiet, signal)
        } else {
          await unsubscribe(signal)
        }
      } catch (error) {
        if (!signal.aborted) {
          reportError(wanted ? t('failures.turnOnNotifications') : t('failures.turnOffNotifications'), error, {
            site: wanted ? 'failures.turnOnNotifications' : 'failures.turnOffNotifications',
          })
        }
      }
    })
  }

  async function subscribe(session: string, events: PushEvents, quiet: boolean, signal: AbortSignal): Promise<void> {
    const registration = await navigator.serviceWorker.ready
    subscribed = await subscribeFor(session, {
      current: () => registration.pushManager.getSubscription(),
      make: async () => {
        const { publicKey } = await readResponse(await apiRequest('/push/key', { signal }), pushKeySchema)
        return registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: publicKey })
      },
      put: async (subscription) => {
        await apiRequest('/push/subscription', { method: 'PUT', signal, ...jsonBody(subscriptionBody(subscription, events, quiet)) })
      },
      maker: subscriptionMaker,
      remember: rememberMaker,
    })
    await leaveWords()
  }

  async function unsubscribe(signal: AbortSignal): Promise<void> {
    subscribed = null
    const registration = await navigator.serviceWorker.ready
    const subscription = await registration.pushManager.getSubscription()
    if (!subscription) {
      return
    }
    await apiRequest('/push/subscription', { method: 'DELETE', signal })
    rememberMaker(null)
    await subscription.unsubscribe()
  }

  /**
   * Tells the backend, on the synchronization channel, whether the page is
   * in front of the user and the conversation it shows then: no turn's end
   * notifies this browser while it is, and nothing of that conversation.
   */
  function reportShowing(): void {
    const front = inFront()
    product.show(front ? product.activeConversationId : null, front)
  }

  /** Tells the channel the user acted on the page, while it is visible. */
  function reportUse(): void {
    if (document.visibilityState === 'visible') {
      product.used()
    }
  }

  /** Notifies from now on, until `stop`; a click on a notification calls `open` with its conversation. */
  function start(open: (id: string) => void): void {
    if (lifetime) {
      return
    }
    const current = new AbortController()
    lifetime = current
    const { signal } = current
    // A browser that cannot notify still says it is in use, which holds
    // back the user's other browsers.
    for (const input of [...USER_INPUT_EVENTS, FRAME_INPUT]) {
      window.addEventListener(input, reportUse, { capture: true, passive: true, signal })
    }
    if (permission.value === 'unsupported') {
      return
    }
    // The user may allow or block the site in the browser's settings while the page is open.
    void navigator.permissions?.query({ name: 'notifications' }).then((status) => {
      if (!signal.aborted) {
        status.addEventListener('change', () => { permission.value = browserPermission() }, { signal })
      }
    }, (error: unknown) => {
      // A browser without the query keeps the permission it was asked for.
      pageLog.warn('notification_permission.follow_failed', errorFields(error))
    })
    navigator.serviceWorker.addEventListener('message', (event: MessageEvent<unknown>) => {
      const data = event.data
      if (typeof data === 'object' && data !== null && Reflect.get(data, 'type') === OPEN_CONVERSATION) {
        const id = Reflect.get(data, 'conversationId')
        if (typeof id === 'string') {
          open(id)
        }
      }
    }, { signal })
    // Each subscription the backend keeps says whether its browser has
    // notifications on, in the account's sessions, which the page holds while
    // it notifies.
    const releaseSessions = product.hold('sessions')
    const session = () => product.snapshot?.sessions?.find((candidate) => candidate.current)?.id
    // Each source is compared by its value, so only a change of one follows:
    // every part the channel brings replaces the snapshot, and the sessions
    // come again after each subscription the backend keeps, as each says
    // whether its browser has notifications on.
    const stopFollowing = watch(
      [
        () => settings.value.enabled,
        permission,
        session,
        () => JSON.stringify(eventsOf(settings.value)),
        () => settings.value.quietWhileElsewhere,
      ],
      () => follow(signal, session()),
      { immediate: true },
    )
    const stopWords = watch(pageLanguage, () => {
      if (subscribed) {
        void leaveWords().catch((error: unknown) => pageLog.warn('notification_words.leave_failed', errorFields(error)))
      }
    })
    const stopShowing = watch(() => product.activeConversationId, reportShowing, { immediate: true })
    window.addEventListener('focus', reportShowing, { signal })
    window.addEventListener('blur', reportShowing, { signal })
    document.addEventListener('visibilitychange', reportShowing, { signal })
    signal.addEventListener('abort', () => {
      stopFollowing()
      stopWords()
      stopShowing()
      releaseSessions()
    }, { once: true })
  }

  /** Stops following the page; the browser stays subscribed, as notifications come with no page open. */
  function stop(): void {
    lifetime?.abort()
    lifetime = null
    subscribed = null
  }

  return { settings, permission, homeScreen, requestPermission, start, stop }
})
