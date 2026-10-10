import { computed } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  userPreferencesSchema,
  type CommandLocale,
  type ContextLimitChange,
  type Preferences,
  type PreferencesPatch,
  type ShortcutsPatch,
} from '../api/generated/web-api'
import { useProduct, type Answer, type ProductChange } from './product'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import { appThemeStore } from '@demicodes/web-ui/theme/appTheme'
import { DEFAULT_SEND_WHILE_RUNNING } from '@demicodes/web-ui/agent/send-way'

/**
 * `preferences` with what `patch` changes, as the backend applies it
 * (`web-api.md` § User preferences): appearance keys merge, a shortcut set
 * to null goes back to its default, a context limit of null leaves the
 * model's full window, and every other field replaces the stored one.
 */
function withPatch(preferences: Preferences, patch: PreferencesPatch): Preferences {
  const { appearance, shortcuts, contextLimit, ...fields } = patch
  const next: Preferences = {
    ...preferences,
    ...fields,
    appearance: { ...preferences.appearance, ...appearance },
    shortcuts: { ...preferences.shortcuts },
  }
  for (const [id, keys] of Object.entries(shortcuts ?? {})) {
    const key = id as keyof ShortcutsPatch
    if (keys === null) {
      delete next.shortcuts[key]
    } else if (keys !== undefined) {
      next.shortcuts[key] = keys
    }
  }
  if (contextLimit) {
    const { providerId, modelId, tokens } = contextLimit
    const models = { ...preferences.contextLimits?.[providerId] }
    if (tokens === null) {
      delete models[modelId]
    } else {
      models[modelId] = tokens
    }
    next.contextLimits = { ...preferences.contextLimits, [providerId]: models }
  }
  return next
}

/** `patch` with `later` over it, as one write that makes both changes. */
function mergePatch(patch: PreferencesPatch, later: PreferencesPatch): PreferencesPatch {
  const merged: PreferencesPatch = { ...patch, ...later }
  if (patch.appearance || later.appearance) {
    merged.appearance = { ...patch.appearance, ...later.appearance }
  }
  if (patch.shortcuts || later.shortcuts) {
    merged.shortcuts = { ...patch.shortcuts, ...later.shortcuts }
  }
  return merged
}

export const usePreferences = defineStore('preferences', () => {
  const product = useProduct()
  const writes = new SerialQueue()
  let timer: ReturnType<typeof setTimeout> | null = null
  let controller = new AbortController()
  /**
   * The changes the user made that wait for the pause before their write,
   * which shows them meanwhile (`web-application.md` § Responding to the
   * user), and the one patch that makes them all.
   */
  let queued: { patch: PreferencesPatch; changes: ProductChange[] } = { patch: {}, changes: [] }

  const lastModel = computed(() => product.snapshot?.preferences.lastModel)
  const lastProjectHost = computed(() => product.snapshot?.preferences.lastProjectHost)
  /** What Enter does with a message while the agent works (`product.md` § Steer or queue). */
  const sendWhileRunning = computed(() =>
    product.snapshot?.preferences.sendWhileRunning ?? DEFAULT_SEND_WHILE_RUNNING,
  )

  /**
   * The limit the user stored on the model `modelId` of the entry
   * `providerId`; null for none, which is the model's full window
   * (`models.md` § Context limit).
   */
  function contextLimit(providerId: string, modelId: string): number | null {
    return product.snapshot?.preferences.contextLimits?.[providerId]?.[modelId] ?? null
  }

  /** Saves `patch`; answers what the backend stored. */
  async function save(patch: PreferencesPatch, signal: AbortSignal, keepalive = false): Promise<Answer> {
    const response = await apiRequest('/settings/preferences', {
      method: 'PATCH',
      keepalive,
      ...jsonBody(patch),
      signal,
    })
    const { preferences } = await readResponse(response, userPreferencesSchema)
    return { type: 'preferences', preferences }
  }

  /** Shows `patch` of the preferences at once, until its write lands. */
  function show(patch: PreferencesPatch): ProductChange {
    return product.change('preferences', (state) => ({
      ...state,
      preferences: withPatch(state.preferences, patch),
    }))
  }

  const appearance = computed(() => ({
    theme: 'system' as const,
    tone: 'ink' as const,
    accent: 'blue' as const,
    fontSize: 15,
    ...product.snapshot?.preferences.appearance,
  }))
  const keys = computed(() =>
    APP_SHORTCUTS.map((binding) => ({
      ...binding,
      keys: product.snapshot?.preferences.shortcuts[binding.id] ?? binding.keys,
    })),
  )

  /**
   * Changes the preferences as `patch` says, at once; the write follows once
   * the user pauses, or at once when `immediate`.
   */
  function update(patch: PreferencesPatch, immediate = false): void {
    queued = {
      patch: mergePatch(queued.patch, patch),
      changes: [...queued.changes, show(patch)],
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

  /** Sends the changes waiting for their write, as one, after the earlier writes. */
  async function flush(): Promise<void> {
    const current = controller
    await writes.run(async () => {
      if (current.signal.aborted || !queued.changes.length) {
        return
      }
      const { patch, changes } = queued
      queued = { patch: {}, changes: [] }
      for (const change of changes) {
        change.send()
      }
      try {
        const answer = await save(patch, current.signal, true)
        for (const change of changes) {
          change.land(answer)
        }
      } catch (error) {
        for (const change of changes) {
          change.drop()
        }
        if (!current.signal.aborted) {
          reportError('Could Not Update Settings', error, { userVisible: true })
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
        const sentAt = product.sent()
        product.answered(sentAt, await save(patch, current.signal))
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
   * with it; the menu shows it at once.
   */
  async function setContextLimit(change: ContextLimitChange): Promise<void> {
    const patch = { contextLimit: change }
    const shown = show(patch)
    const current = controller
    await writes.run(async () => {
      shown.send()
      try {
        shown.land(await save(patch, current.signal))
      } catch (error) {
        shown.drop()
        if (!current.signal.aborted) {
          reportError('Could Not Change the Context Limit', error, { userVisible: true })
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
    for (const change of queued.changes) {
      change.drop()
    }
    queued = { patch: {}, changes: [] }
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
