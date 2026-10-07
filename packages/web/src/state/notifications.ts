import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import { z } from 'zod'
import { truncate } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { markdownPlainText } from '@demicodes/web-ui/markdown/plain-text'
import { categoryAction } from '@demicodes/web-ui/permissions/types'
import { readPermissions } from '../api/permissions'
import type { ConversationSummary, LastTurn } from '../api/generated/web-api'
import { useProduct } from './product'

/**
 * Notifications (`product.md` § Notifications): the browser's own, shown
 * while a conversation is not in front, of what the page's synchronization
 * channel tells about it.
 */

const settingsSchema = z.object({
  /** The user turned notifications on in this browser. */
  enabled: z.boolean(),
  turnFinishes: z.boolean(),
  turnFails: z.boolean(),
  needsPermission: z.boolean(),
})
export type NotificationSettings = z.infer<typeof settingsSchema>

/** Off until turned on; once on, everything notifies. */
const DEFAULT_SETTINGS: NotificationSettings = {
  enabled: false,
  turnFinishes: true,
  turnFails: true,
  needsPermission: true,
}

/** The setting belongs to the browser, not the account, so one key holds it for every user of the browser. */
const STORAGE_KEY = 'demi.notifications'

/** How much of the answer a notification says. */
const ANSWER_START_CHARS = 120

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

/** What the browser allows this site; `unsupported` for one without notifications. */
export type BrowserPermission = NotificationPermission | 'unsupported'

function browserPermission(): BrowserPermission {
  return typeof Notification === 'undefined' ? 'unsupported' : Notification.permission
}

/** What a change of a conversation's summary notifies of. */
export type NotificationCause =
  | { kind: 'turnFinished'; turn: LastTurn }
  | { kind: 'turnFailed'; turn: LastTurn }
  | { kind: 'permission' }

/**
 * What the page holds of a conversation it follows: its latest summary, and
 * the ended turn it has accounted for, notified or not.
 */
export interface Followed {
  summary: ConversationSummary
  endedTurn: string | null
}

/**
 * The page's next hold of a conversation, from what it held before and the
 * conversation's `next` summary, and what that summary notifies of under
 * `settings`, while `front` is the conversation in front of the user, or
 * null when the page is hidden or not focused.
 *
 * A turn notifies once its conversation's work has settled and the summary
 * names an ended turn other than the one the page accounted for, so a turn
 * that starts and ends between two summaries still notifies. While the
 * conversation runs, its latest ended turn may be an older one, as a retry
 * or an edit cuts the last one from the history, so the page waits for the
 * work to settle. A turn that ended with an answer or an error notifies; one
 * the user stopped does not. A change of the permission requests that leaves
 * some undecided asks for a read of them, to tell the new ones. A
 * conversation the page has not followed before has no change to tell.
 */
export function follow(
  previous: Followed | undefined,
  next: ConversationSummary,
  settings: NotificationSettings,
  front: string | null,
): { followed: Followed; causes: NotificationCause[] } {
  const turn = next.lastTurn
  if (!previous) {
    return { followed: { summary: next, endedTurn: turn?.id ?? null }, causes: [] }
  }
  const settled = next.status !== 'running' && next.status !== 'compacting'
  const followed = { summary: next, endedTurn: settled ? (turn?.id ?? null) : previous.endedTurn }
  if (!settings.enabled || next.id === front) {
    return { followed, causes: [] }
  }
  const causes: NotificationCause[] = []
  if (settled && turn && turn.id !== previous.endedTurn) {
    if (turn.outcome === 'finished' && settings.turnFinishes) {
      causes.push({ kind: 'turnFinished', turn })
    }
    if (turn.outcome === 'failed' && settings.turnFails) {
      causes.push({ kind: 'turnFailed', turn })
    }
  }
  if (
    settings.needsPermission &&
    next.permissionRequests > 0 &&
    next.permissionsRevision !== previous.summary.permissionsRevision
  ) {
    causes.push({ kind: 'permission' })
  }
  return { followed, causes }
}

/**
 * The start of a finished turn's answer as a notification says it: in plain
 * text, cut to 120 characters; empty for a turn that ended without one.
 */
export function answerStart(turn: LastTurn): string {
  return turn.answerStart === null ? '' : truncate(markdownPlainText(turn.answerStart), ANSWER_START_CHARS)
}

/** The tag of a turn's notification: the same on every page of the browser, which shows it once. */
function turnTag(conversationId: string, turn: LastTurn): string {
  return `turn:${conversationId}:${turn.id}`
}

/**
 * This browser's notification settings and what the browser allows, and,
 * from `start` to `stop`, the notifications themselves: each change of a
 * conversation's summary that the settings ask for, while the conversation
 * is not in front, becomes the browser's notification, and a click on it
 * brings the page forward with the conversation open.
 */
export const useNotifications = defineStore('notifications', () => {
  const product = useProduct()
  const settings = ref(readSettings())
  watch(settings, writeSettings, { deep: true })
  const permission = ref<BrowserPermission>(browserPermission())

  /** Asks the browser for permission to notify; a browser that blocks the site answers without asking. */
  async function requestPermission(): Promise<NotificationPermission> {
    if (typeof Notification === 'undefined') {
      return 'denied'
    }
    const answer = await Notification.requestPermission()
    permission.value = answer
    return answer
  }

  /** Set from `start` to `stop`. */
  let lifetime: AbortController | null = null
  let stopFollowing: (() => void) | null = null
  /** The notifications shown and not yet closed, which `stop` closes. */
  const shown = new Set<Notification>()
  /** The undecided permission requests already notified, by conversation. */
  const notifiedRequests = new Map<string, Set<string>>()

  /** The conversation in front of the user; none while the page is hidden or not focused. */
  function front(): string | null {
    return document.visibilityState === 'visible' && document.hasFocus()
      ? product.activeConversationId
      : null
  }

  function show(conversationId: string, title: string, body: string, tag: string, open: (id: string) => void): void {
    if (!lifetime || !settings.value.enabled || browserPermission() !== 'granted' || front() === conversationId) {
      return
    }
    let notification: Notification
    try {
      notification = new Notification(title, { body, tag })
    } catch (error) {
      // A browser that shows notifications only through a service worker,
      // such as Chrome on Android, refuses the constructor; the page
      // registers none for them, so it shows nothing there.
      console.warn('Could not show a notification', error)
      return
    }
    shown.add(notification)
    notification.addEventListener('close', () => shown.delete(notification), { once: true })
    notification.addEventListener('click', () => {
      window.focus()
      open(conversationId)
      notification.close()
    }, { once: true })
  }

  async function notify(cause: NotificationCause, summary: ConversationSummary, open: (id: string) => void): Promise<void> {
    const signal = lifetime?.signal
    if (!signal) {
      return
    }
    switch (cause.kind) {
      case 'turnFinished':
        show(summary.id, summary.title, answerStart(cause.turn), turnTag(summary.id, cause.turn), open)
        return
      case 'turnFailed':
        show(summary.id, summary.title, 'The turn stopped with an error.', turnTag(summary.id, cause.turn), open)
        return
      case 'permission': {
        let requests
        try {
          requests = (await readPermissions(summary.id)).requests
        } catch (error) {
          if (!signal.aborted) {
            reportError('Could Not Read the Permission Requests for a Notification', error)
          }
          return
        }
        if (signal.aborted || !settings.value.needsPermission) {
          return
        }
        const notified = notifiedRequests.get(summary.id) ?? new Set<string>()
        // Only the requests still undecided are remembered.
        notifiedRequests.set(summary.id, new Set(requests.map((request) => request.id)))
        for (const request of requests) {
          if (!notified.has(request.id)) {
            const action = categoryAction({ id: request.category.id, action: request.category.action ?? null, description: null })
            show(summary.id, summary.title, `Allow this conversation to ${action}?`, `permission:${request.id}`, open)
          }
        }
        return
      }
    }
  }

  /** Notifies from now on, until `stop`; a click on a notification calls `open` with its conversation. */
  function start(open: (id: string) => void): void {
    if (lifetime) {
      return
    }
    const current = new AbortController()
    lifetime = current
    // The user may allow or block the site in the browser's settings while the page is open.
    void navigator.permissions?.query({ name: 'notifications' }).then((status) => {
      if (!current.signal.aborted) {
        status.addEventListener('change', () => { permission.value = browserPermission() }, { signal: current.signal })
      }
    }, (error: unknown) => {
      // A browser without the query keeps the permission it was asked for.
      console.warn('Could not follow the notification permission', error)
    })
    let following = new Map<string, Followed>()
    const followAll = (summaries: readonly ConversationSummary[] | undefined) => {
      const inFront = front()
      const next = new Map<string, Followed>()
      for (const summary of summaries ?? []) {
        const { followed, causes } = follow(following.get(summary.id), summary, settings.value, inFront)
        next.set(summary.id, followed)
        for (const cause of causes) {
          void notify(cause, summary, open)
        }
      }
      following = next
    }
    followAll(product.snapshot?.conversations)
    stopFollowing = watch(() => product.snapshot?.conversations, followAll)
  }

  function stop(): void {
    lifetime?.abort()
    lifetime = null
    stopFollowing?.()
    stopFollowing = null
    for (const notification of shown) {
      notification.close()
    }
    shown.clear()
    notifiedRequests.clear()
  }

  return { settings, permission, requestPermission, start, stop }
})
