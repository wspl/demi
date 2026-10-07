<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { z } from 'zod'
import SettingsSkills from './SettingsSkills.vue'
import type { AddSourceAnswer, SettingsSkillDraft, SettingsSkillSource } from './types'
import { PluginCallError, pendingCalls, usePage, type HeadlineText } from '@demicodes/plugin-sdk'
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
const calls = pendingCalls(page.errors)

/** Calls `method` for `source`, which answers nothing; a refusal says why in a toast. */
function change(
  source: string,
  method: string,
  params: SourceCall | SetEnabled | SetSourceEnabled,
  couldNot: HeadlineText,
): void {
  void calls.run(source, couldNot, () => plugin.call(method, params, z.null()))
}

/** The refusals that are about the origin typed, which the dialog says under its field (`skills.md` § The page). */
const ORIGIN_REFUSALS = ['invalid_origin', 'source_exists']

async function add(draft: SettingsSkillDraft): Promise<AddSourceAnswer> {
  try {
    await plugin.call('add_source', { origin: draft.origin } satisfies AddSource, addedSourceSchema)
    return { kind: 'added' }
  } catch (error) {
    if (error instanceof PluginCallError && ORIGIN_REFUSALS.includes(error.reason)) {
      return { kind: 'refused', message: error.message }
    }
    page.errors.report('Could Not Add the Source', error)
    return { kind: 'failed' }
  }
}
</script>

<template>
  <SettingsSkills
    v-model:open="openIds"
    :sources="sources"
    :pending="calls.pending.value"
    :overlay-store="page.overlays"
    :add-source="add"
    @update="(source) => change(source, 'update_source', { source }, 'Could Not Update the Source')"
    @remove="(source) => change(source, 'remove_source', { source }, 'Could Not Remove the Source')"
    @switch="(source, skill, enabled) => change(source, 'set_enabled', { source, skill, enabled }, 'Could Not Switch the Skill')"
    @switch-source="(source, enabled) => change(source, 'set_source_enabled', { source, enabled }, 'Could Not Switch the Skills')"
  />
</template>
