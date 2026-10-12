import { onScopeDispose, ref, watch, type Ref } from 'vue'
import type { SettingsModelMatch, SettingsModelSearch } from './types'

/** How long typing pauses before the search is asked. */
export const MODEL_SEARCH_DELAY_MS = 150

/**
 * The matches for what the Add Model dialog's field holds. Typing asks again
 * after a pause; a search still on its way when the text changes is aborted
 * and its answer never shown, so the list is always the latest text's. An
 * empty text has no matches. The text the field starts with is searched at once. A failed search leaves the list empty and says so
 * in `failed`, since *Custom* still works.
 */
export function useModelSearch(
  query: Ref<string>,
  search: () => SettingsModelSearch | undefined,
  delay = MODEL_SEARCH_DELAY_MS,
) {
  const results = ref<SettingsModelMatch[]>([])
  const failed = ref(false)
  let timer: ReturnType<typeof setTimeout> | null = null
  let controller: AbortController | null = null

  function stop(): void {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    controller?.abort()
    controller = null
  }

  watch(query, (text) => {
    stop()
    const trimmed = text.trim()
    const ask = search()
    if (!trimmed || !ask) {
      results.value = []
      failed.value = false
      return
    }
    const current = new AbortController()
    controller = current
    timer = setTimeout(() => {
      timer = null
      ask(trimmed, current.signal).then(
        (matches) => {
          if (!current.signal.aborted) {
            results.value = matches
            failed.value = false
          }
        },
        () => {
          // An aborted search is a stale one; any other failure leaves Custom.
          if (!current.signal.aborted) {
            results.value = []
            failed.value = true
          }
        },
      )
    }, delay)
  }, { immediate: true })

  onScopeDispose(stop)
  return { results, failed }
}
