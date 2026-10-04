import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  userPreferencesSchema,
  type CommandLocale,
  type ContextLimitChange,
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

  /** Context limits sent and not answered yet, oldest first, which the menu shows at once. */
  const pendingLimits = ref<ContextLimitChange[]>([])

  const lastModel = computed(() =>
    pending.value.lastModel ?? product.snapshot?.preferences.lastModel,
  )
  const lastProjectHost = computed(() =>
    pending.value.lastProjectHost ??
    product.snapshot?.preferences.lastProjectHost,
  )

  /**
   * The limit the user stored on the model `modelId` of the entry
   * `providerId`, the latest one in flight first; null for none, which is
   * the model's full window (`models.md` § Context limit).
   */
  function contextLimit(providerId: string, modelId: string): number | null {
    const sent = pendingLimits.value
      .filter((change) => change.providerId === providerId && change.modelId === modelId)
      .at(-1)
    if (sent) {
      return sent.tokens
    }
    return product.snapshot?.preferences.contextLimits?.[providerId]?.[modelId] ?? null
  }

  /** Saves `patch` and hands the stored preferences to the product state. */
  async function save(patch: PreferencesPatch, signal: AbortSignal, keepalive = false): Promise<void> {
    const sentAt = product.sent()
    const response = await apiRequest('/settings/preferences', {
      method: 'PATCH',
      keepalive,
      ...jsonBody(patch),
      signal,
    })
    const { preferences } = await readResponse(response, userPreferencesSchema)
    product.answered(sentAt, { type: 'preferences', preferences })
  }

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
        ...(pending.value.lastProjectHost
          ? { lastProjectHost: pending.value.lastProjectHost }
          : {}),
        appearance: { ...pending.value.appearance },
        shortcuts: { ...pending.value.shortcuts },
      } satisfies PreferencesPatch
      if (
        !Object.keys(patch.appearance).length &&
        !Object.keys(patch.shortcuts).length &&
        !patch.lastModel &&
        !patch.lastProjectHost
      ) {
        return
      }
      try {
        await save(patch, current.signal, true)
      } catch (error) {
        if (!current.signal.aborted) {
          reportError('Could not update settings', error, { userVisible: true })
        }
      } finally {
        if (!current.signal.aborted) {
          if (pending.value.lastModel === patch.lastModel) {
            delete pending.value.lastModel
          }
          if (pending.value.lastProjectHost === patch.lastProjectHost) {
            delete pending.value.lastProjectHost
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
   * Sends the web browser's time zone and languages whenever they differ from
   * the stored ones; commands receive them (`web-api.md` § User preferences).
   */
  async function reportLocale(): Promise<void> {
    const stored = product.snapshot?.preferences
    const locale = webBrowserLocale()
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
        await save({ locale }, current.signal)
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

  /**
   * Stores the user's context limit on one model, for every conversation
   * with it; the menu shows it before the backend answers.
   */
  async function setContextLimit(change: ContextLimitChange): Promise<void> {
    pendingLimits.value = [...pendingLimits.value, change]
    const current = controller
    await writes.run(async () => {
      try {
        await save({ contextLimit: change }, current.signal)
      } catch (error) {
        if (!current.signal.aborted) {
          reportError('Could not change the context limit', error, { userVisible: true })
        }
      } finally {
        pendingLimits.value = pendingLimits.value.filter((sent) => sent !== change)
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
    pendingLimits.value = []
  }

  return {
    lastModel,
    lastProjectHost,
    contextLimit,
    setContextLimit,
    appearance,
    keys,
    update,
    flush,
    reportLocale,
    stop,
  }
})

/**
 * The web browser's own time zone and languages, as the backend stores them;
 * null when it reports neither.
 */
function webBrowserLocale(): CommandLocale | null {
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const reported = navigator.languages?.length ? navigator.languages : [navigator.language]
  try {
    const languages = Intl.getCanonicalLocales(reported.filter(Boolean)).slice(0, 16)
    return timeZone && languages.length > 0 ? { timeZone, languages } : null
  } catch {
    return null
  }
}
