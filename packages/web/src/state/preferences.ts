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
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import { appThemeStore } from '@demicodes/web-ui/theme/appTheme'
import { DEFAULT_SEND_WHILE_RUNNING } from '@demicodes/web-ui/agent/send-way'

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
  /** What Enter does with a message while the agent works (`product.md` § Steer or queue). */
  const sendWhileRunning = computed(() =>
    pending.value.sendWhileRunning ??
    product.snapshot?.preferences.sendWhileRunning ??
    DEFAULT_SEND_WHILE_RUNNING,
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
    APP_SHORTCUTS.map((binding) => {
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
        ...(pending.value.sendWhileRunning
          ? { sendWhileRunning: pending.value.sendWhileRunning }
          : {}),
        appearance: { ...pending.value.appearance },
        shortcuts: { ...pending.value.shortcuts },
      } satisfies PreferencesPatch
      if (
        !Object.keys(patch.appearance).length &&
        !Object.keys(patch.shortcuts).length &&
        !patch.lastModel &&
        !patch.lastProjectHost &&
        !patch.sendWhileRunning
      ) {
        return
      }
      try {
        await save(patch, current.signal, true)
      } catch (error) {
        if (!current.signal.aborted) {
          reportError('Could Not Update Settings', error, { userVisible: true })
        }
      } finally {
        if (!current.signal.aborted) {
          if (pending.value.lastModel === patch.lastModel) {
            delete pending.value.lastModel
          }
          if (pending.value.lastProjectHost === patch.lastProjectHost) {
            delete pending.value.lastProjectHost
          }
          if (pending.value.sendWhileRunning === patch.sendWhileRunning) {
            delete pending.value.sendWhileRunning
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

  /** The report in flight, so a second trigger does not repeat it. */
  let reporting: string | null = null
  /**
   * This page's last report as it sent it and as the backend stored it, in
   * its canonical spelling: a stored value that is the answer to the page's
   * own facts is not a difference to report.
   */
  let reported: { sent: string; stored: string } | null = null
  /**
   * Sends the web browser's time zone and languages, and the color scheme the
   * page shows, whichever differ from the stored ones; commands receive them,
   * and the conversation browser starts with them (`web-api.md` § User
   * preferences). The caller calls it when the page's own facts may have
   * changed, never because the stored ones did.
   */
  async function reportBrowser(): Promise<void> {
    const stored = product.snapshot?.preferences
    if (!stored) {
      return
    }
    const locale = webBrowserLocale()
    const sentLocale = locale && JSON.stringify(locale)
    const storedLocale = JSON.stringify(stored.locale ?? null)
    // The backend's canonical spelling of what this page sent is the same locale.
    const sameLocale = sentLocale === storedLocale
      || (reported !== null && reported.sent === sentLocale && reported.stored === storedLocale)
    const colorScheme = appThemeStore.state.mode
    const patch: PreferencesPatch = {
      ...(locale && !sameLocale ? { locale } : {}),
      ...(colorScheme !== stored.colorScheme ? { colorScheme } : {}),
    }
    const key = JSON.stringify(patch)
    if (Object.keys(patch).length === 0 || key === reporting) {
      return
    }
    reporting = key
    const current = controller
    await writes.run(async () => {
      try {
        await save(patch, current.signal)
        if (patch.locale)
          reported = { sent: JSON.stringify(patch.locale), stored: JSON.stringify(product.snapshot?.preferences.locale ?? null) }
      } catch (error) {
        if (!current.signal.aborted) {
          reportError('Could Not Report the Time Zone, Languages and Color Scheme', error)
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
          reportError('Could Not Change the Context Limit', error, { userVisible: true })
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
    sendWhileRunning,
    contextLimit,
    setContextLimit,
    appearance,
    keys,
    update,
    flush,
    reportBrowser,
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
