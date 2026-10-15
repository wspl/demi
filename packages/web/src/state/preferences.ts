import { computed } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { t } from './locales/text'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  userPreferencesSchema,
  type BrowserLocale,
  type ContextLimitChange,
  type Preferences,
  type PreferencesPatch,
  type ShortcutsPatch,
} from '../api/generated/web-api'
import { useProduct, type Answer, type ProductChange } from './product'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import { appThemeStore } from '@demicodes/web-ui/theme/appTheme'
import { resolveLanguage } from '@demicodes/web-ui/i18n/languages'
import { DEFAULT_SEND_WHILE_RUNNING } from '@demicodes/web-ui/agent/send-way'
import { DEFAULT_ACCENT, DEFAULT_TRANSCRIPT_TEXT_SIZE } from '@demicodes/web-ui/theme/productAppearance'

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
  /**
   * The preference writes on their way, which go one at a time in order:
   * the backend answers each with the whole document, so the document is
   * the one thing they change. Only a flush as the page goes passes them.
   */
  const writes = new SerialQueue()
  let timer: ReturnType<typeof setTimeout> | null = null
  let controller = new AbortController()
  /**
   * The changes the user made that wait for their write, behind a write in
   * flight or for the pause after a bursty control, which shows them
   * meanwhile (`web-application.md` § Responding to the user), and the one
   * patch that makes them all.
   */
  let queued: { patch: PreferencesPatch; changes: ProductChange[] } = { patch: {}, changes: [] }

  const lastModel = computed(() => product.snapshot?.preferences.lastModel)
  const lastProjectHost = computed(() => product.snapshot?.preferences.lastProjectHost)
  /** The language the user chose (`localization.md` § Choosing the language); absent means System. */
  const language = computed(() => product.snapshot?.preferences.language ?? 'system')
  /** What Enter does with a message while the agent works (`product.md` § Steer or queue). */
  const sendWhileRunning = computed(() =>
    product.snapshot?.preferences.sendWhileRunning ?? DEFAULT_SEND_WHILE_RUNNING,
  )
  /**
   * Whether a turn that failed with a known time to try again resumes by
   * itself at that time (`web-api.md` § User preferences); absent means no.
   */
  const resumeAtRetry = computed(() => product.snapshot?.preferences.resumeAtRetry ?? false)
  /**
   * Whether a turn a deployment, an upgrade or a restart of the backend
   * interrupted goes on by itself once the backend is back (`web-api.md`
   * § User preferences); absent means yes.
   */
  const resumeAfterRestart = computed(() => product.snapshot?.preferences.resumeAfterRestart ?? true)
  /**
   * Whether the user went on past the warning about several subscription
   * accounts, which then never shows again (`providers.md` § Account
   * rotation); absent means no.
   */
  const multiAccountWarned = computed(() => product.snapshot?.preferences.multiAccountWarned ?? false)

  /**
   * Whether the user turned the provider entry `providerId` off in
   * Settings › Providers, which takes it out of every model menu
   * (`web-api.md` § User preferences).
   */
  function hidesProvider(providerId: string): boolean {
    return product.snapshot?.preferences.hiddenProviders?.includes(providerId) ?? false
  }

  /** Whether the user turned the model `modelId` of the entry `providerId` off. */
  function hidesModel(providerId: string, modelId: string): boolean {
    return product.snapshot?.preferences.hiddenModels?.some(
      (hidden) => hidden.providerId === providerId && hidden.modelId === modelId,
    ) ?? false
  }

  /** Turns the entry `providerId` on or off in every model menu, at once. */
  function showProvider(providerId: string, shown: boolean): void {
    if (hidesProvider(providerId) !== shown) {
      return
    }
    const hidden = (product.snapshot?.preferences.hiddenProviders ?? [])
      .filter((candidate) => candidate !== providerId)
    update({ hiddenProviders: shown ? hidden : [...hidden, providerId] })
  }

  /** Turns the model `modelId` of the entry `providerId` on or off in every model menu, at once. */
  function showModel(providerId: string, modelId: string, shown: boolean): void {
    if (hidesModel(providerId, modelId) !== shown) {
      return
    }
    const hidden = (product.snapshot?.preferences.hiddenModels ?? [])
      .filter((candidate) => candidate.providerId !== providerId || candidate.modelId !== modelId)
    update({ hiddenModels: shown ? hidden : [...hidden, { providerId, modelId }] })
  }

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
    accent: DEFAULT_ACCENT,
    fontSize: DEFAULT_TRANSCRIPT_TEXT_SIZE,
    ...product.snapshot?.preferences.appearance,
  }))
  const keys = computed(() =>
    APP_SHORTCUTS.map((binding) => ({
      ...binding,
      keys: product.snapshot?.preferences.shortcuts[binding.id] ?? binding.keys,
    })),
  )

  /**
   * Changes the preferences as `patch` says, at once, and writes the change
   * after the writes before it (`web-api.md` § User preferences): at once,
   * as for a switch, or once the user pauses when the control is `bursty`,
   * as a slider is.
   */
  function update(patch: PreferencesPatch, { bursty = false } = {}): void {
    queued = {
      patch: mergePatch(queued.patch, patch),
      changes: [...queued.changes, show(patch)],
    }
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    const current = controller
    const write = () => void writes.run('preferences', () => sendWaiting(current))
    if (!bursty) {
      write()
      return
    }
    timer = setTimeout(() => {
      timer = null
      write()
    }, 200)
  }

  /**
   * Sends the changes still waiting for their write at once, as the page is
   * hidden, reloaded or closed: not after the user's pause, nor behind a
   * write in flight, which would hold it past the page's end. Its request
   * starts before this returns, and the web browser sends it after the page
   * is gone.
   */
  function flush(): Promise<void> {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    return sendWaiting(controller)
  }

  /**
   * Writes the changes waiting for their write as one, with `keepalive` so
   * that the page's end does not cancel it; nothing when none wait, as when
   * a flush sent them already.
   */
  async function sendWaiting(current: AbortController): Promise<void> {
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
        reportError(t('failures.updateSettings'), error, { userVisible: true })
      }
    }
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
   * Sends the web browser's time zone and languages with the language the
   * page resolves from them, and the color scheme the
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
    await writes.run('preferences', async () => {
      try {
        const sentAt = product.sent()
        product.answered(sentAt, await save(patch, current.signal))
        if (patch.locale)
          reported = { sent: JSON.stringify(patch.locale), stored: JSON.stringify(product.snapshot?.preferences.locale ?? null) }
      } catch (error) {
        if (!current.signal.aborted) {
          reportError(t('failures.updateTimeZoneLanguage'), error)
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
    await writes.run('preferences', async () => {
      shown.send()
      try {
        shown.land(await save(patch, current.signal))
      } catch (error) {
        shown.drop()
        if (!current.signal.aborted) {
          reportError(t('failures.changeContextLimit'), error, { userVisible: true })
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
    language,
    lastModel,
    lastProjectHost,
    sendWhileRunning,
    resumeAtRetry,
    resumeAfterRestart,
    multiAccountWarned,
    contextLimit,
    setContextLimit,
    hidesProvider,
    hidesModel,
    showProvider,
    showModel,
    appearance,
    keys,
    update,
    flush,
    reportBrowser,
    stop,
  }
})

/**
 * The web browser's own time zone and languages, as the backend stores them,
 * and the language the page resolves for System from them, which the
 * backend's mail reads rather than resolving it again (`localization.md` §
 * Choosing the language); null when the browser reports neither.
 */
function webBrowserLocale(): BrowserLocale | null {
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const reported = navigator.languages?.length ? navigator.languages : [navigator.language]
  try {
    const languages = Intl.getCanonicalLocales(reported.filter(Boolean)).slice(0, 16)
    return timeZone && languages.length > 0
      ? { timeZone, languages, language: resolveLanguage('system', languages) }
      : null
  } catch {
    return null
  }
}
