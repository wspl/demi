import { computed, getCurrentScope, onScopeDispose, ref } from 'vue'
import { requestFailure } from './request-failures'
import type { ShownError } from '@demicodes/web-ui/infra/errors'
import type { UsageLimits } from '@demicodes/web-ui/agent/context-usage'
import type { QuotaKind, QuotaWindowName } from '@demicodes/protocol'
import { DAY, HOUR, MINUTE, ago, ahead } from './time'

/** Seconds in an hour, a day and a week, which a window's period counts. */
export const HOUR_SECONDS = 3600
export const DAY_SECONDS = 24 * HOUR_SECONDS
export const WEEK_SECONDS = 7 * DAY_SECONDS

/**
 * What names a window as a vendor reports it: its kind, its period in
 * seconds (null when the vendor does not say) and the model it is on, which
 * the page words in its language (`packages/web-ui/src/ui/dates.ts`).
 */
export function windowNamed(kind: QuotaKind, periodSeconds: number | null, model: string | null = null): QuotaWindowName {
  return { kind, periodSeconds, scope: model === null ? null : { kind: 'model', label: model } }
}

/**
 * What one card specimen keeps of its account's usage limits, as the product's
 * store does: the windows with when they were reported, whether a manual
 * refresh runs and the vendor's words for one that failed. Refresh usage
 * takes a moment and reads each window a little further used, reported now;
 * showing the card asks nothing. A refresh under way ends with the specimen.
 */
export function useCardLimits(accountLabel: string, failure: string | null = null) {
  const windows = ref<UsageLimits['windows']>([
    { id: 'five_hour', name: windowNamed('limit', 5 * HOUR_SECONDS), used: 18, max: 100, resetsAt: ahead(3 * HOUR + 38 * MINUTE) },
    { id: 'seven_day', name: windowNamed('limit', WEEK_SECONDS), used: 34, max: 100, resetsAt: ahead(5 * DAY + 2 * HOUR) },
    { id: 'limit:weekly_scoped:Fable', name: windowNamed('limit', WEEK_SECONDS, 'Fable'), used: 81, max: 100, resetsAt: ahead(12 * DAY) },
  ])
  const observedAt = ref(ago(6 * MINUTE))
  const refreshing = ref(false)
  const error = ref<ShownError | null>(null)
  let timer: number | undefined
  if (getCurrentScope()) {
    onScopeDispose(() => window.clearTimeout(timer))
  }
  const limits = computed<UsageLimits>(() => ({
    accountLabel,
    windows: windows.value,
    observedAt: observedAt.value,
    refreshing: refreshing.value,
    error: error.value,
  }))
  function answer(): void {
    windows.value = windows.value.map((window) => ({ ...window, used: Math.min(window.max, window.used + 1) }))
    observedAt.value = new Date().toISOString()
    error.value = null
  }
  function refresh(): void {
    if (refreshing.value) {
      return
    }
    refreshing.value = true
    timer = window.setTimeout(() => {
      refreshing.value = false
      if (failure) {
        error.value = requestFailure(failure)
      } else {
        answer()
      }
    }, 1200)
  }
  return { limits, refresh }
}
