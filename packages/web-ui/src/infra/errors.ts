import { reactive } from 'vue'

export interface Toast {
  id: string
  title: string
  message: string
}

export interface ReportErrorOptions {
  userVisible?: boolean
  expected?: boolean
  detail?: string
}

export const toasts = reactive<Toast[]>([])

export function reportError(title: string, error: unknown, options: ReportErrorOptions = {}): void {
  const message = error instanceof Error ? error.message : String(error)
  if (!options.expected) console.error(`[demi] ${title}: ${message}`, options.detail ?? '')
  if (!options.userVisible) return
  const id = globalThis.crypto.randomUUID()
  toasts.push({ id, title, message })
  setTimeout(() => dismissToast(id), 6000)
}

export function dismissToast(id: string): void {
  const index = toasts.findIndex((toast) => toast.id === id)
  if (index >= 0) toasts.splice(index, 1)
}
