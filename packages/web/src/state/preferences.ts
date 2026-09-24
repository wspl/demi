import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  userPreferencesSchema,
  type CommandLocale,
  type PreferencesPatch,
} from '../api/generated/web-api'
import { useProduct } from './product'

export const DEFAULT_KEYS = [
  {
    id: 'new',
    action: 'New conversation',
    keys: '⌘N',
  },
  {
    id: 'sidebar',
    action: 'Toggle sidebar',
    keys: '⌘B',
  },
  {
    id: 'settings',
    action: 'Open settings',
    keys: '⌘,',
  },
] as const

export const usePreferences = defineStore('preferences', () => {
  const product = useProduct()
  const pending = ref<PreferencesPatch>({})
  const writes = new SerialQueue()
  let timer: ReturnType<typeof setTimeout> | null = null
  let controller = new AbortController()

  const lastModel = computed(() =>
    pending.value.lastModel ?? product.snapshot?.preferences.lastModel,
  )

  const appearance = computed(() => ({
    theme: 'system' as const,
    tone: 'ink' as const,
    accent: 'blue' as const,
    fontSize: 15,
    ...product.snapshot?.preferences.appearance,
    ...pending.value.appearance,
  }))
  const keys = computed(() =>
    DEFAULT_KEYS.map((binding) => {
      const queued = pending.value.shortcuts?.[binding.id]
      return {
        ...binding,
        keys:
          queued === null
            ? binding.keys
            : (queued ??
              product.snapshot?.preferences.shortcuts[binding.id] ??
              binding.keys),
      }
    }),
  )

  function update(patch: PreferencesPatch, immediate = false): void {
    pending.value = {
      ...pending.value,
      ...patch,
      appearance: {
        ...pending.value.appearance,
        ...patch.appearance,
      },
      shortcuts: {
        ...pending.value.shortcuts,
        ...patch.shortcuts,
      },
    }
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    if (immediate) {
      void flush()
      return
    }
    timer = setTimeout(() => {
      timer = null
      void flush()
    }, 200)
  }

  async function flush(): Promise<void> {
    const current = controller
    await writes.run(async () => {
      if (current.signal.aborted) {
        return
      }
      const patch = {
        ...(pending.value.lastModel ? { lastModel: pending.value.lastModel } : {}),
        appearance: { ...pending.value.appearance },
        shortcuts: { ...pending.value.shortcuts },
      } satisfies PreferencesPatch
      if (
        !Object.keys(patch.appearance).length &&
        !Object.keys(patch.shortcuts).length &&
        !patch.lastModel
      ) {
        return
      }
      try {
        const response = await apiRequest('/settings/preferences', {
          method: 'PATCH',
          keepalive: true,
          ...jsonBody(patch),
          signal: current.signal,
        })
        await readResponse(response, userPreferencesSchema)
        await product.refresh()
      } catch (error) {
        if (!current.signal.aborted) {
          reportError('Could not update settings', error, { userVisible: true })
        }
      } finally {
        if (!current.signal.aborted) {
          if (pending.value.lastModel === patch.lastModel) {
            delete pending.value.lastModel
          }
          for (const field of ['appearance', 'shortcuts'] as const) {
            const queued = pending.value[field] as Record<string, unknown>
            for (const [key, value] of Object.entries(patch[field])) {
              if (queued?.[key] === value) {
                delete queued[key]
              }
            }
          }
        }
      }
    })
  }

  /** The locale a report in flight sends, so a second trigger does not repeat it. */
  let reporting: string | null = null
  /**
   * Sends the browser's time zone and languages whenever they differ from the
   * stored ones; commands receive them (`web-api.md` § User preferences).
   */
  async function reportLocale(): Promise<void> {
    const stored = product.snapshot?.preferences
    const locale = browserLocale()
    if (!stored || !locale) {
      return
    }
    const key = JSON.stringify(locale)
    if (key === JSON.stringify(stored.locale ?? null) || key === reporting) {
      return
    }
    reporting = key
    const current = controller
    await writes.run(async () => {
      try {
        const response = await apiRequest('/settings/preferences', {
          method: 'PATCH',
          ...jsonBody({ locale } satisfies PreferencesPatch),
          signal: current.signal,
        })
        await readResponse(response, userPreferencesSchema)
        await product.refresh()
      } catch (error) {
        if (!current.signal.aborted) {
          reportError('Could not report the time zone and languages', error)
        }
      } finally {
        if (reporting === key) {
          reporting = null
        }
      }
    })
  }

  function stop(): void {
    reporting = null
    controller.abort()
    controller = new AbortController()
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = null
    pending.value = {}
  }

  return {
    lastModel,
    appearance,
    keys,
    update,
    flush,
    reportLocale,
    stop,
  }
})

/**
 * The browser's own time zone and languages, as the backend stores them;
 * null when the browser reports neither.
 */
function browserLocale(): CommandLocale | null {
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const reported = navigator.languages?.length ? navigator.languages : [navigator.language]
  try {
    const languages = Intl.getCanonicalLocales(reported.filter(Boolean)).slice(0, 16)
    return timeZone && languages.length > 0 ? { timeZone, languages } : null
  } catch {
    return null
  }
}
