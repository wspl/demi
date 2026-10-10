import { computed } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import type { SettingsSubagentDraft } from '@demicodes/web-ui/settings/types'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  profileAnswerSchema,
  type NewProfile,
  type ProfilePatch,
  type SubagentProfile,
  type SubagentSettings,
  type SubagentSwitch,
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
 * The user's subagent settings as the Subagent section changes them
 * (`web-api.md` § Subagents). A switch shows at once and its write follows
 * the earlier ones (`web-application.md` § Responding to the user); a saved
 * or deleted profile shows its answer. The channel brings the same part to
 * every other page.
 */
export const useSubagentSettings = defineStore('subagent-settings', () => {
  const product = useProduct()
  const writes = new SerialQueue()
  const settings = computed(() => product.snapshot?.subagents ?? null)

  /**
   * Shows `next` of the settings at once and sends `write` after the earlier
   * switches; the change lands with what `write` answers, or leaves with a
   * toast titled `failed`.
   */
  function switchSettings(
    next: (current: SubagentSettings) => SubagentSettings,
    write: () => Promise<Answer | undefined>,
    failed: HeadlineText,
  ): Promise<void> {
    const change = product.change('subagents', (state) => ({ ...state, subagents: next(state.subagents) }))
    return writes
      .run(async () => {
        change.send()
        change.land(await write())
      })
      .catch((error) => {
        change.drop()
        reportError(failed, error, { userVisible: true })
      })
  }

  function switchSubagents(enabled: boolean): Promise<void> {
    return switchSettings(
      (current) => ({ ...current, enabled }),
      async () => {
        await apiRequest('/subagents', {
          method: 'PUT',
          ...jsonBody({ enabled } satisfies SubagentSwitch),
        })
        return undefined
      },
      enabled ? 'Could Not Turn Subagents On' : 'Could Not Turn Subagents Off',
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
      (current) => ({
        ...current,
        profiles: current.profiles.map((profile) => profile.id === id ? { ...profile, enabled } : profile),
      }),
      async () => {
        const profile = await patchProfile(id, { enabled })
        return settingsAnswer((current) => withProfile(current, profile))
      },
      enabled ? 'Could Not Enable the Profile' : 'Could Not Disable the Profile',
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
    await apiRequest(`/subagents/profiles/${encodeURIComponent(id)}`, { method: 'DELETE' })
    product.answered(at, settingsAnswer((current) => ({
      ...current,
      profiles: current.profiles.filter((profile) => profile.id !== id),
    })))
  }

  return {
    settings,
    switchSubagents,
    switchProfile,
    saveProfile,
    deleteProfile,
  }
})
