import { reactive } from 'vue'
import { createId } from '@demicodes/utils'

/**
 * A toast is only for an outcome that has nowhere else to go: the menu already
 * closed, the drop has no field, the send failed off-control. Do not toast a
 * success the page already shows, or stand in for a dialog that does not exist.
 */
export type ToastTone = 'danger' | 'neutral'

export interface Toast {
  id: string
  title: string
  message?: string
  tone: ToastTone
}

export const TOAST_DURATION_MS = 6000

export const toasts = reactive<Toast[]>([])

const timers = new Map<string, ReturnType<typeof setTimeout>>()

export function showToast(input: {
  title: string
  message?: string
  tone?: ToastTone
  durationMs?: number
}): string {
  const id = createId()
  toasts.push({
    id,
    title: input.title,
    message: input.message,
    tone: input.tone ?? 'neutral',
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
  if (index >= 0) toasts.splice(index, 1)
}
