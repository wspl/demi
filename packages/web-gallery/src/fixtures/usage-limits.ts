import { computed, ref } from 'vue'
import type { UsageLimits } from '@demicodes/web-ui/agent/context-usage'
import { DAY, HOUR, MINUTE, ahead } from './time'

/**
 * What one card specimen keeps of its account's usage limits, as the product's
 * store does: the windows, whether a manual refresh runs and the vendor's
 * words for one that failed. A probe takes a moment and reads each window a
 * little further used; the automatic one, run when the card shows, is silent.
 */
export function useCardLimits(accountLabel: string, failure: string | null = null) {
  const windows = ref<UsageLimits['windows']>([
    { id: 'five_hour', label: '5h session', used: 18, max: 100, resetsAt: ahead(3 * HOUR + 38 * MINUTE) },
    { id: 'seven_day', label: '7d all models', used: 34, max: 100, resetsAt: ahead(5 * DAY + 2 * HOUR) },
    { id: 'limit:weekly_scoped:Fable', label: '7d Fable', used: 81, max: 100, resetsAt: ahead(12 * DAY) },
  ])
  const refreshing = ref(false)
  const error = ref<string | null>(null)
  const limits = computed<UsageLimits>(() => ({
    accountLabel,
    autoRefresh: true,
    windows: windows.value,
    refreshing: refreshing.value,
    error: error.value,
  }))
  function answer(): void {
    windows.value = windows.value.map((window) => ({ ...window, used: Math.min(window.max, window.used + 1) }))
    error.value = null
  }
  function refresh(automatic: boolean): void {
    if (automatic) {
      answer()
      return
    }
    if (refreshing.value) {
      return
    }
    refreshing.value = true
    window.setTimeout(() => {
      refreshing.value = false
      if (failure) {
        error.value = failure
      } else {
        answer()
      }
    }, 1200)
  }
  return { limits, refresh }
}
