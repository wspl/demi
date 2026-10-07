import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import { z } from 'zod'
import { truncate } from '@demicodes/utils'
import type { Block } from '@demicodes/protocol'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { markdownPlainText } from '@demicodes/web-ui/markdown/plain-text'
import { categoryAction } from '@demicodes/web-ui/permissions/types'
import { apiRequest, readResponse } from '../api/client'
import { readPermissions } from '../api/permissions'
import { transcriptSchema, type ConversationSummary } from '../api/generated/web-api'
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
export type NotificationCause = 'turnFinished' | 'turnFailed' | 'permission'

/**
 * What the change from `previous` to `next` of a conversation's summary
 * notifies of, under `settings`, while `front` is the conversation in front
 * of the user, or null when the page is hidden or not focused. A turn that
 * ended with an answer or an error notifies; one the user stopped does not.
 * A change of the permission requests that leaves some undecided asks for a
 * read of them, to tell the new ones. A conversation the page has seen no
 * summary of before has no change to tell.
 */
export function notificationCauses(
  previous: ConversationSummary | undefined,
  next: ConversationSummary,
  settings: NotificationSettings,
  front: string | null,
): NotificationCause[] {
  if (!settings.enabled || !previous || next.id === front) {
    return []
  }
  const causes: NotificationCause[] = []
  if (previous.status === 'running' && next.status === 'completed' && settings.turnFinishes) {
    causes.push('turnFinished')
  }
  if (previous.status === 'running' && next.status === 'error' && settings.turnFails) {
    causes.push('turnFailed')
  }
  if (
    settings.needsPermission &&
    next.permissionRequests > 0 &&
    next.permissionsRevision !== previous.permissionsRevision
  ) {
    causes.push('permission')
  }
  return causes
}

/**
 * The start of the answer that ended the last turn, as a notification says
 * it: the last text after the user's last message, in plain text, cut to
 * 120 characters; empty for a turn that ended without one.
 */
export function answerStart(blocks: readonly Block[]): string {
  const asked = blocks.findLastIndex((block) => block.type === 'user')
  const answer = blocks.slice(asked + 1).findLast((block) => block.type === 'text')
  return answer?.type === 'text' ? truncate(markdownPlainText(answer.text), ANSWER_START_CHARS) : ''
}

function byId(summaries: readonly ConversationSummary[] | undefined): Map<string, ConversationSummary> {
  return new Map((summaries ?? []).map((summary) => [summary.id, summary]))
}

/** The tag of a turn's notification: the same on every page of the browser, which shows it once. */
function turnTag(summary: ConversationSummary): string {
  return `turn:${summary.id}:${summary.revision}`
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
    switch (cause) {
      case 'turnFinished': {
        // Once the conversation no longer runs, its transcript shows what
        // the live tree showed (`web-api.md` § Sidebar mutations and read state).
        let body = ''
        try {
          const response = await apiRequest(`/conversations/${encodeURIComponent(summary.id)}/transcript`, { signal })
          body = answerStart((await readResponse(response, transcriptSchema)).blocks)
        } catch (error) {
          if (signal.aborted) {
            return
          }
          // The notification still says that the turn finished, without the answer.
          reportError('Could Not Read the Answer for a Notification', error)
        }
        if (settings.value.turnFinishes) {
          show(summary.id, summary.title, body, turnTag(summary), open)
        }
        return
      }
      case 'turnFailed':
        show(summary.id, summary.title, 'The turn stopped with an error.', turnTag(summary), open)
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
    let previous = byId(product.snapshot?.conversations)
    stopFollowing = watch(
      () => product.snapshot?.conversations,
      (summaries) => {
        const inFront = front()
        for (const summary of summaries ?? []) {
          for (const cause of notificationCauses(previous.get(summary.id), summary, settings.value, inFront)) {
            void notify(cause, summary, open)
          }
        }
        previous = byId(summaries)
      },
    )
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
