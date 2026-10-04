<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { z } from 'zod'
import SettingsSkills from './SettingsSkills.vue'
import type { SettingsSkillDraft, SettingsSkillSource } from './types'
import { usePage } from '@demicodes/plugin-sdk'
import {
  addedSourceSchema,
  skillsStateSchema,
  type AddSource,
  type CheckUpdates,
  type SetEnabled,
  type SetSourceEnabled,
  type SourceCall,
} from './generated/plugin'

/**
 * The skills plugin's settings section (`skills.md` § The page): its
 * Skills page over the plugin's state and methods. Every change comes back
 * with the state the plugin sends to each of the user's pages. Opening it
 * asks the plugin to check the sources for updates.
 */
const props = defineProps<{
  /** The sources shown open at first. */
  open?: readonly string[]
}>()

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
    updateAvailable: source.updateAvailable,
    skills: source.skills.map((skill) => ({ ...skill, takenBy: skill.takenBy ?? undefined })),
    skipped: source.skipped,
  })),
)

const openIds = ref<string[]>([...(props.open ?? [])])

onMounted(() => {
  plugin.call('check_updates', {} satisfies CheckUpdates, z.null()).catch((error: unknown) => {
    // Nothing waits for it: a check that cannot start shows nothing new.
    page.errors.defect('Could not check the sources for updates', error)
  })
})

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
    v-model:open="openIds"
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
