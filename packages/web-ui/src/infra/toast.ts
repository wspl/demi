import { reactive } from 'vue'
import { createId } from '@demicodes/utils'
import type { HeadlineText, SentenceText, TitleText } from '../ui/ui-text'

/**
 * A toast is only for an outcome that has nowhere else to go: the menu already
 * closed, the drop has no field, the send failed off-control. Do not toast a
 * success the page already shows, or stand in for a dialog that does not exist.
 *
 * Its tone says what happened, and its mark follows: `success`, the action did
 * what was asked (Copied); `neutral`, a fact about the request, which neither
 * worked nor failed (a folder outside the workspace); `danger`, it failed. Every
 * toast names its tone, so none claims a success by default.
 *
 * A toast may offer one action, such as Reload, which closes it.
 *
 * A failure stays until it is closed; any other toast closes by itself after
 * `TOAST_DURATION_MS`, counted only while the pointer is not over the toasts.
 */
export type ToastTone = 'success' | 'neutral' | 'danger'

export interface ToastAction {
  label: TitleText
  run(): void
}

export interface Toast {
  id: string
  title: HeadlineText
  message?: SentenceText
  tone: ToastTone
  action?: ToastAction
}

/** How long a toast that is not a failure stays while the pointer is not over the toasts. */
export const TOAST_DURATION_MS = 6000

export const toasts = reactive<Toast[]>([])

/**
 * The countdown of each toast that closes by itself: its timer while it runs,
 * and the time it has left. A failure has none, since the reader may need
 * longer than any countdown to read it, as Slack and Linear keep theirs.
 */
interface Countdown {
  timer: ReturnType<typeof setTimeout> | null
  remainingMs: number
  startedAt: number
}

const countdowns = new Map<string, Countdown>()
/** The pointer is over the toasts, which holds every countdown where it is. */
let held = false

function run(id: string, countdown: Countdown): void {
  countdown.startedAt = Date.now()
  countdown.timer = setTimeout(() => dismissToast(id), countdown.remainingMs)
}

export function showToast(input: {
  title: HeadlineText
  message?: SentenceText
  tone: ToastTone
  action?: ToastAction
}): string {
  const id = createId()
  toasts.push({
    id,
    title: input.title,
    message: input.message,
    tone: input.tone,
    action: input.action,
  })
  if (input.tone !== 'danger') {
    const countdown: Countdown = { timer: null, remainingMs: TOAST_DURATION_MS, startedAt: 0 }
    countdowns.set(id, countdown)
    if (!held) {
      run(id, countdown)
    }
  }
  return id
}

/** The pointer entered the toasts: every countdown stops where it is. */
export function holdToasts(): void {
  if (held) {
    return
  }
  held = true
  const now = Date.now()
  for (const countdown of countdowns.values()) {
    if (countdown.timer !== null) {
      clearTimeout(countdown.timer)
      countdown.timer = null
      countdown.remainingMs = Math.max(0, countdown.remainingMs - (now - countdown.startedAt))
    }
  }
}

/** The pointer left the toasts: every countdown goes on from where it stopped. */
export function releaseToasts(): void {
  if (!held) {
    return
  }
  held = false
  for (const [id, countdown] of countdowns) {
    run(id, countdown)
  }
}

export function dismissToast(id: string): void {
  const countdown = countdowns.get(id)
  if (countdown?.timer != null) {
    clearTimeout(countdown.timer)
  }
  countdowns.delete(id)
  const index = toasts.findIndex((toast) => toast.id === id)
  if (index >= 0)
    toasts.splice(index, 1)
  // A toast closed under the pointer takes its leave event with it.
  if (!toasts.length) {
    held = false
  }
}
