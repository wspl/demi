import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
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
import { useProduct } from '../state/product'

/** `settings` with `profile` in its place by id, the profiles in name order. */
function withProfile(settings: SubagentSettings, profile: SubagentProfile): SubagentSettings {
  const others = settings.profiles.filter((candidate) => candidate.id !== profile.id)
  const profiles = [...others, profile].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
  return { ...settings, profiles }
}

/**
 * The user's subagent settings as the Subagent section changes them
 * (`web-api.md` § Subagents). Each write's answer goes into the product
 * state at once; the channel brings the same part to every other page.
 */
export const useSubagentSettings = defineStore('subagent-settings', () => {
  const product = useProduct()
  const switching = ref(false)
  /** The profiles whose switch is being saved. */
  const pending = ref<string[]>([])
  const settings = computed(() => product.snapshot?.subagents ?? null)

  /** Applies the subagent settings `next` makes of the current ones, as a write sent `at` left them. */
  function answered(at: number, next: (current: SubagentSettings) => SubagentSettings): void {
    const current = settings.value
    if (current) {
      product.answered(at, { type: 'subagents', subagents: next(current) })
    }
  }

  async function switchSubagents(enabled: boolean): Promise<void> {
    if (switching.value) {
      return
    }
    switching.value = true
    try {
      const at = product.sent()
      await apiRequest('/subagents', {
        method: 'PUT',
        ...jsonBody({ enabled } satisfies SubagentSwitch),
      })
      answered(at, (current) => ({ ...current, enabled }))
    } catch (error) {
      reportError(enabled ? 'Could Not Turn Subagents On' : 'Could Not Turn Subagents Off', error, {
        userVisible: true,
      })
    } finally {
      switching.value = false
    }
  }

  /** Sends `patch` of the profile `id` and applies the profile it answers. */
  async function patchProfile(id: string, patch: ProfilePatch): Promise<void> {
    const at = product.sent()
    const response = await apiRequest(`/subagents/profiles/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      ...jsonBody(patch),
    })
    const { profile } = await readResponse(response, profileAnswerSchema)
    answered(at, (current) => withProfile(current, profile))
  }

  async function switchProfile(id: string, enabled: boolean): Promise<void> {
    if (pending.value.includes(id)) {
      return
    }
    pending.value.push(id)
    try {
      await patchProfile(id, { enabled })
    } catch (error) {
      reportError(enabled ? 'Could Not Enable the Profile' : 'Could Not Disable the Profile', error, {
        userVisible: true,
      })
    } finally {
      pending.value = pending.value.filter((candidate) => candidate !== id)
    }
  }

  /** Creates a profile, or changes the profile `id`; a refusal is thrown for the editor to show. */
  async function saveProfile(id: string | null, draft: SettingsSubagentDraft): Promise<void> {
    if (id !== null) {
      await patchProfile(id, draft)
      return
    }
    const at = product.sent()
    const response = await apiRequest('/subagents/profiles', {
      method: 'POST',
      ...jsonBody(draft satisfies NewProfile),
    })
    const { profile } = await readResponse(response, profileAnswerSchema)
    answered(at, (current) => withProfile(current, profile))
  }

  async function deleteProfile(id: string): Promise<void> {
    const at = product.sent()
    await apiRequest(`/subagents/profiles/${encodeURIComponent(id)}`, { method: 'DELETE' })
    answered(at, (current) => ({
      ...current,
      profiles: current.profiles.filter((profile) => profile.id !== id),
    }))
  }

  return {
    settings,
    switching,
    pending,
    switchSubagents,
    switchProfile,
    saveProfile,
    deleteProfile,
  }
})
