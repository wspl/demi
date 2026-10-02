<script setup lang="ts">
import { computed, ref } from 'vue'
import { z } from 'zod'
import SettingsSkills from './SettingsSkills.vue'
import type { SettingsSkillDraft, SettingsSkillSource } from './types'
import { usePage } from '@demicodes/plugin-sdk'
import {
  addedSourceSchema,
  skillsStateSchema,
  type AddSource,
  type SetEnabled,
  type SetSourceEnabled,
  type SourceCall,
} from './generated/plugin'

/**
 * The skills plugin's settings section (`skills.md` § The page): its
 * Skills page over the plugin's state and methods. Every change comes back
 * with the state the plugin sends to each of the user's pages.
 */
const page = usePage()
const plugin = page.plugin
const state = plugin.state(skillsStateSchema)

const sources = computed<SettingsSkillSource[]>(() =>
  (state.value?.sources ?? []).map((source) => ({
    id: source.id,
    origin: source.origin,
    commit: source.commit ?? undefined,
    fetching: source.fetching,
    failure: source.failure ?? undefined,
    skills: source.skills,
    skipped: source.skipped,
  })),
)

/** The sources with a call in flight, whose controls wait for it. */
const pending = ref<string[]>([])

/** Calls `method` for `source`, which answers nothing; a refusal says why in a toast. */
async function change(
  source: string,
  method: string,
  params: SourceCall | SetEnabled | SetSourceEnabled,
  couldNot: string,
): Promise<void> {
  if (pending.value.includes(source)) {
    return
  }
  pending.value = [...pending.value, source]
  try {
    await plugin.call(method, params, z.null())
  } catch (error) {
    page.errors.report(couldNot, error)
  } finally {
    pending.value = pending.value.filter((id) => id !== source)
  }
}

async function add(draft: SettingsSkillDraft): Promise<void> {
  try {
    await plugin.call('add_source', { origin: draft.origin } satisfies AddSource, addedSourceSchema)
  } catch (error) {
    page.errors.report('Could not add the source', error)
  }
}
</script>

<template>
  <SettingsSkills
    :sources="sources"
    :pending="pending"
    :overlay-store="page.overlays"
    @add="add"
    @update="(source) => change(source, 'update_source', { source }, 'Could not update the source')"
    @remove="(source) => change(source, 'remove_source', { source }, 'Could not remove the source')"
    @switch="(source, skill, enabled) => change(source, 'set_enabled', { source, skill, enabled }, 'Could not switch the skill')"
    @switch-source="(source, enabled) => change(source, 'set_source_enabled', { source, enabled }, 'Could not switch the skills')"
  />
</template>
