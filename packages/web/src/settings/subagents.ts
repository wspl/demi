import { t } from './locales/text'
import { computed } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { errorCode, reportError } from '@demicodes/web-ui/infra/errors'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import type { SettingsSubagentDraft } from '@demicodes/web-ui/settings/types'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  profileAnswerSchema,
  type NewProfile,
  type ProfilePatch,
  type SubagentLimit,
  type SubagentProfile,
  type SubagentSettings,
  type SubagentsPatch,
} from '../api/generated/web-api'
import { useProduct, type Answer } from '../state/product'

/** `settings` with `profile` in its place by id, the profiles in name order. */
function withProfile(settings: SubagentSettings, profile: SubagentProfile): SubagentSettings {
  const others = settings.profiles.filter((candidate) => candidate.id !== profile.id)
  const profiles = [...others, profile].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
  return { ...settings, profiles }
}

/** The answer that leaves the subagent settings as `next` makes them of those the page last read. */
function settingsAnswer(next: (current: SubagentSettings) => SubagentSettings): Answer {
  return (read) => ({ type: 'subagents', subagents: next(read.subagents) })
}

/**
 * The user's subagent settings as the Subagents section changes them. A switch
 * or a limit shows at once and its write follows the earlier ones of the same
 * setting: the Subagents switch, a limit, or one profile's switch; a saved or
 * deleted profile shows its answer. The channel brings the same part to every
 * other page.
 */
export const useSubagentSettings = defineStore('subagent-settings', () => {
  const product = useProduct()
  /** The writes on their way, by what they set: `subagents`, a limit's name, or `profile:<id>`. */
  const writes = new SerialQueue()
  const settings = computed(() => product.snapshot?.subagents ?? null)

  /**
   * Shows `next` of the settings at once and sends `write` after the earlier
   * switches of `key`; the change lands with what `write` answers, or leaves with a
   * toast titled `failed`.
   */
  function switchSettings(
    key: string,
    next: (current: SubagentSettings) => SubagentSettings,
    write: () => Promise<Answer | undefined>,
    failed: HeadlineText,
  ): Promise<void> {
    const change = product.change('subagents', (state) => ({ ...state, subagents: next(state.subagents) }))
    return writes
      .run(key, async () => {
        change.send()
        change.land(await write())
      })
      .catch((error) => {
        change.drop()
        // A profile another page deleted meanwhile has nothing to switch,
        // and the channel takes it off the list.
        if (errorCode(error) !== 'profile_not_found') {
          reportError(failed, error, { userVisible: true })
        }
      })
  }

  /** Sends `patch` of the switch and the limits, which answers nothing. */
  async function putSubagents(patch: SubagentsPatch): Promise<undefined> {
    await apiRequest('/subagents', { method: 'PUT', ...jsonBody(patch) })
    return undefined
  }

  function switchSubagents(enabled: boolean): Promise<void> {
    return switchSettings(
      'subagents',
      (current) => ({ ...current, enabled }),
      () => putSubagents({ enabled }),
      enabled ? t('subagents.turnOnFailed') : t('subagents.turnOffFailed'),
    )
  }

  /** Sets Maximum subagents or Maximum depth; Unlimited, which keeps the number, the page confirmed. */
  function setLimit(limit: 'maxLive' | 'maxDepth', value: SubagentLimit): Promise<void> {
    return switchSettings(
      limit,
      (current) => ({ ...current, [limit]: value }),
      () => putSubagents({ [limit]: value }),
      t('subagents.limitFailed'),
    )
  }

  /** Sends `patch` of the profile `id`; answers the profile it answers. */
  async function patchProfile(id: string, patch: ProfilePatch): Promise<SubagentProfile> {
    const response = await apiRequest(`/subagents/profiles/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      ...jsonBody(patch),
    })
    return (await readResponse(response, profileAnswerSchema)).profile
  }

  function switchProfile(id: string, enabled: boolean): Promise<void> {
    return switchSettings(
      `profile:${id}`,
      (current) => ({
        ...current,
        profiles: current.profiles.map((profile) => profile.id === id ? { ...profile, enabled } : profile),
      }),
      async () => {
        const profile = await patchProfile(id, { enabled })
        return settingsAnswer((current) => withProfile(current, profile))
      },
      enabled ? t('subagents.enableFailed') : t('subagents.disableFailed'),
    )
  }

  /** Creates a profile, or changes the profile `id`; a refusal is thrown for the editor to show. */
  async function saveProfile(id: string | null, draft: SettingsSubagentDraft): Promise<void> {
    const at = product.sent()
    let profile: SubagentProfile
    if (id !== null) {
      profile = await patchProfile(id, draft)
    } else {
      const response = await apiRequest('/subagents/profiles', {
        method: 'POST',
        ...jsonBody(draft satisfies NewProfile),
      })
      profile = (await readResponse(response, profileAnswerSchema)).profile
    }
    product.answered(at, settingsAnswer((current) => withProfile(current, profile)))
  }

  async function deleteProfile(id: string): Promise<void> {
    const at = product.sent()
    try {
      await apiRequest(`/subagents/profiles/${encodeURIComponent(id)}`, { method: 'DELETE' })
    } catch (error) {
      // A profile another page deleted meanwhile is deleted: that is done.
      if (errorCode(error) !== 'profile_not_found') {
        throw error
      }
    }
    product.answered(at, settingsAnswer((current) => ({
      ...current,
      profiles: current.profiles.filter((profile) => profile.id !== id),
    })))
  }

  return {
    settings,
    switchSubagents,
    setLimit,
    switchProfile,
    saveProfile,
    deleteProfile,
  }
})
