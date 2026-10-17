import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import { z } from 'zod'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { conversationTitle } from '@demicodes/web-ui/agent/conversation-title'
import { pageLanguage } from '@demicodes/web-ui/i18n/text'
import { requestTitleWords } from '@demicodes/web-ui/permissions/types'
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
 * Notifications (`turns.md` § Notifications): push notifications, which the
 * browser's push service brings to the page's service worker whether or not
 * a page is open. This browser's settings live in its storage; while they
 * are on and the browser allows them, the page keeps the browser
 * subscribed for its session, with the events it chose, tells the backend
 * on its synchronization channel which conversation it shows, focused and
 * visible, so that conversation is not notified to this browser, and leaves
 * the service worker the words it says notifications in. A click on a
 * notification opens its conversation.
 *
 * A browser's subscription serves one session: signing out unsubscribes the
 * browser, and a session that finds it subscribed by another makes a new
 * subscription before it sends one, so nothing queued for the account
 * signed out before shows after another signs in.
 */

const settingsSchema = z.object({
  /** The user turned notifications on in this browser. */
  enabled: z.boolean(),
  turnFinishes: z.boolean(),
  turnFails: z.boolean(),
  needsPermission: z.boolean(),
  agentNotifies: z.boolean(),
})
export type NotificationSettings = z.infer<typeof settingsSchema>

/** Off until turned on; once on, everything notifies. */
const DEFAULT_SETTINGS: NotificationSettings = {
  enabled: false,
  turnFinishes: true,
  turnFails: true,
  needsPermission: true,
  agentNotifies: true,
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
    console.warn('Could not read the notification settings', error)
    return { ...DEFAULT_SETTINGS }
  }
}

function writeSettings(settings: NotificationSettings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings))
  } catch (error) {
    // Private browsing and storage quotas keep the setting for this page alone.
    console.warn('Could not save the notification settings', error)
  }
}

/** Which session made the browser's subscription, which belongs to the browser too. */
const MAKER_KEY = 'demi.notifications.session'

function subscriptionMaker(): string | null {
  try {
    return localStorage.getItem(MAKER_KEY)
  } catch (error) {
    // Without storage, the page cannot tell: it makes a new subscription.
    console.warn('Could not read which session subscribed the browser', error)
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
    console.warn('Could not save which session subscribed the browser', error)
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
function subscriptionBody(subscription: PushSubscription, events: PushEvents): PushSubscriptionBody {
  return {
    endpoint: subscription.endpoint,
    keys: pushSubscriptionKeysSchema.parse(subscription.toJSON().keys),
    events,
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
    console.warn('Could not unsubscribe the browser from notifications', error)
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

  /** The conversation in front of the user; none while the page is hidden or not focused. */
  function front(): string | null {
    return document.visibilityState === 'visible' && document.hasFocus()
      ? product.activeConversationId
      : null
  }

  /**
   * Keeps the browser subscribed with the events it chose while
   * notifications are on and allowed, and unsubscribed otherwise; one
   * change at a time, in order.
   */
  function follow(signal: AbortSignal, session: string | undefined): void {
    const wanted = settings.value.enabled && permission.value === 'granted'
    const events = eventsOf(settings.value)
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
          await subscribe(session, events, signal)
        } else {
          await unsubscribe(signal)
        }
      } catch (error) {
        if (!signal.aborted) {
          reportError(wanted ? t('failures.turnOnNotifications') : t('failures.turnOffNotifications'), error)
        }
      }
    })
  }

  async function subscribe(session: string, events: PushEvents, signal: AbortSignal): Promise<void> {
    const registration = await navigator.serviceWorker.ready
    subscribed = await subscribeFor(session, {
      current: () => registration.pushManager.getSubscription(),
      make: async () => {
        const { publicKey } = await readResponse(await apiRequest('/push/key', { signal }), pushKeySchema)
        return registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: publicKey })
      },
      put: async (subscription) => {
        await apiRequest('/push/subscription', { method: 'PUT', signal, ...jsonBody(subscriptionBody(subscription, events)) })
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

  /** Tells the backend, on the synchronization channel, the conversation this page shows, focused and visible. */
  function reportShowing(): void {
    product.show(front())
  }

  /** Notifies from now on, until `stop`; a click on a notification calls `open` with its conversation. */
  function start(open: (id: string) => void): void {
    if (lifetime || permission.value === 'unsupported') {
      return
    }
    const current = new AbortController()
    lifetime = current
    const { signal } = current
    // The user may allow or block the site in the browser's settings while the page is open.
    void navigator.permissions?.query({ name: 'notifications' }).then((status) => {
      if (!signal.aborted) {
        status.addEventListener('change', () => { permission.value = browserPermission() }, { signal })
      }
    }, (error: unknown) => {
      // A browser without the query keeps the permission it was asked for.
      console.warn('Could not follow the notification permission', error)
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
    const session = () => product.snapshot?.sessions.find((candidate) => candidate.current)?.id
    const stopFollowing = watch(
      () => [settings.value.enabled, permission.value, session(), ...Object.values(eventsOf(settings.value))],
      () => follow(signal, session()),
      { immediate: true },
    )
    const stopWords = watch(pageLanguage, () => {
      if (subscribed) {
        void leaveWords().catch((error: unknown) => console.warn('Could not leave the notification words', error))
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
