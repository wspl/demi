<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref } from 'vue'
import { z } from 'zod'
import SettingsSkills from './SettingsSkills.vue'
import type { AddSourceAnswer, SettingsSkillDraft, SettingsSkillSource } from './types'
import { formatBytes, PluginCallError, usePage, type HeadlineText } from '@demicodes/plugin-sdk'
import {
  addedSourceSchema,
  skillsStateSchema,
  type AddSource,
  type SourceState,
  type SetEnabled,
  type SetSourceEnabled,
  type Failure,
  type SourceCall,
  type Skipped,
} from './generated/plugin'

/**
 * The skills plugin's settings section: its Skills page over the plugin's
 * state and methods, showing Global's sources and those of the tab's
 * workspace; another workspace's stay out of sight. Every change comes back
 * with the state the plugin sends to each of the user's pages, an update the
 * backend found included (`crates/plugin-skills/plugin/src/heads.rs`):
 * opening it checks nothing.
 */
const props = defineProps<{
  /** The sources shown open at first. */
  open?: readonly string[]
}>()

const page = usePage()
const plugin = page.plugin
const state = plugin.state(skillsStateSchema)

/**
 * Why the last fetch failed, in the page's words: one of Demi's limits or
 * finds, or git's or the system's own words, as they came.
 */
function fetchFailure(failure: Failure): string {
  const problem = failure.problem
  switch (problem?.code) {
    case 'repository_too_large':
    case 'files_too_large':
      return t(`problems.fetch.${problem.code}`, { limit: formatBytes(problem.limitBytes) })
    case 'no_commit':
    case 'no_skill':
      return t(`problems.fetch.${problem.code}`)
    case 'failed':
    case undefined:
      return failure.message
  }
}

/** Why a SKILL.md is not a skill, in the page's words; its English for one recorded before codes were. */
function skipReason(skipped: Skipped): string {
  const problem = skipped.problem
  if (!problem) {
    return skipped.reason
  }
  return problem.code === 'too_large'
    ? t('problems.skip.too_large', { limit: formatBytes(problem.limitBytes) })
    : t(`problems.skip.${problem.code}`)
}

/** A place's name: Global, or its workspace's in the page's language. */
function placeName(workspace: string | null | undefined): string {
  if (!workspace) {
    return t('places.global')
  }
  return page.workspaces.list().find((each) => each.id === workspace)?.name ?? ''
}

function shown(source: SourceState): SettingsSkillSource {
  return {
    id: source.id,
    origin: source.origin,
    commit: source.commit ?? undefined,
    fetching: source.fetching,
    failure: source.failure ? { at: source.failure.at, message: fetchFailure(source.failure) } : undefined,
    updateAvailable: source.updateAvailable,
    skills: source.skills.map((skill) => ({
      ...skill,
      takenBy: skill.takenBy ?? undefined,
      takenIn: skill.takenBy ? placeName(skill.takenIn) : undefined,
    })),
    skipped: source.skipped.map((skipped) => ({ path: skipped.path, reason: skipReason(skipped) })),
  }
}

const all = computed(() => state.value?.sources ?? [])
const global = computed(() => all.value.filter((source) => !source.workspace).map(shown))
const workspace = computed(() => {
  const current = page.workspaces.current()
  return current
    ? { id: current, name: placeName(current), sources: all.value.filter((source) => source.workspace === current).map(shown) }
    : undefined
})

const openIds = ref<string[]>([...(props.open ?? [])])

/**
 * Calls `method`, which answers nothing; a refusal says why in a toast. Its
 * control's operation (`page.operation`) runs until the call is answered.
 */
function change(method: string, params: SourceCall | SetEnabled | SetSourceEnabled, couldNot: HeadlineText): Promise<void> {
  return plugin.call(method, params, z.null()).then(
    () => {},
    (error: unknown) => page.errors.report(couldNot, error),
  )
}

/**
 * A repository as the user may spell it, `owner/repo` or an https URL, in the
 * one form the plugin keys sources by (`crates/plugin-skills/plugin/src/origin.rs`):
 * the URL with its host in lowercase, without a trailing slash or `.git`.
 */
function repository(origin: string): string {
  const text = origin.trim()
  const url = text.startsWith('https://') ? text : `https://github.com/${text}`
  const rest = url.slice('https://'.length)
  const slash = rest.indexOf('/')
  const host = (slash < 0 ? rest : rest.slice(0, slash)).toLowerCase()
  const path = (slash < 0 ? '' : rest.slice(slash + 1)).replace(/\/+$/, '').replace(/\.git$/, '')
  return `https://${host}/${path}`
}

/** Why `origin` is added already, naming the place of the source the page lists for it. */
function exists(origin: string): string {
  const listed = all.value.find((source) => repository(source.origin) === repository(origin))
  return listed
    ? t('refusals.source_exists_in', { origin, place: placeName(listed.workspace) })
    : t('refusals.source_exists', { origin })
}

/**
 * The refusals that are about the origin typed, which the dialog says under
 * its field, in the page's words.
 */
const ORIGIN_REFUSALS = new Map<string, (origin: string) => string>([
  ['invalid_origin', () => t('refusals.invalid_origin')],
  ['source_exists', exists],
  ['workspace_not_found', () => t('refusals.workspace_not_found')],
])

async function add(draft: SettingsSkillDraft, workspace: string | null): Promise<AddSourceAnswer> {
  const params: AddSource = { origin: draft.origin }
  if (workspace) {
    params.workspace = workspace
  }
  try {
    await plugin.call('add_source', params, addedSourceSchema)
    return { kind: 'added' }
  } catch (error) {
    const refusal = error instanceof PluginCallError ? ORIGIN_REFUSALS.get(error.reason) : undefined
    if (refusal) {
      return { kind: 'refused', message: refusal(draft.origin.trim()) }
    }
    page.errors.report(t('failures.add'), error)
    return { kind: 'failed' }
  }
}
</script>

<template>
  <SettingsSkills
    v-model:open="openIds"
    :global="global"
    :workspace="workspace"
    :operation="page.operation"
    :overlay-store="page.overlays"
    :add-source="add"
    @update="(source) => change('update_source', { source }, t('failures.update'))"
    @remove="(source) => change('remove_source', { source }, t('failures.remove'))"
    @switch="(source, skill, enabled) => change('set_enabled', { source, skill, enabled }, t('failures.switchSkill'))"
    @switch-source="(source, enabled) => change('set_source_enabled', { source, enabled }, t('failures.switchSkills'))"
  />
</template>
