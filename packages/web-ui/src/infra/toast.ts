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

export const TOAST_DURATION_MS = 6000

export const toasts = reactive<Toast[]>([])

const timers = new Map<string, ReturnType<typeof setTimeout>>()

export function showToast(input: {
  title: HeadlineText
  message?: SentenceText
  tone: ToastTone
  /** How long it stays; 0 keeps it until it is closed. */
  durationMs?: number
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
  const duration = input.durationMs ?? TOAST_DURATION_MS
  if (duration > 0) {
    timers.set(id, setTimeout(() => dismissToast(id), duration))
  }
  return id
}

export function dismissToast(id: string): void {
  const timer = timers.get(id)
  if (timer != null) {
    clearTimeout(timer)
    timers.delete(id)
  }
  const index = toasts.findIndex((toast) => toast.id === id)
  if (index >= 0)
    toasts.splice(index, 1)
}
