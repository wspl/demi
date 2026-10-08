<script setup lang="ts">
import { computed, ref } from 'vue'
import { GitBranch, RefreshCw, Trash2 } from '@lucide/vue'
import type { OverlayStore, SettingsRowStatus } from '@demicodes/plugin-sdk'
import { Fold } from '@demicodes/plugin-sdk'
import { FoldChevron } from '@demicodes/plugin-sdk'
import { IconButton } from '@demicodes/plugin-sdk'
import { Switch } from '@demicodes/plugin-sdk'
import { Tooltip } from '@demicodes/plugin-sdk'
import { Button } from '@demicodes/plugin-sdk'
import { ConfirmDialog } from '@demicodes/plugin-sdk'
import { ScrollArea } from '@demicodes/plugin-sdk'
import { ICON_PX } from '@demicodes/plugin-sdk'
import AddSkillSourceDialog from './AddSkillSourceDialog.vue'
import { SettingsGroup } from '@demicodes/plugin-sdk'
import { SettingsPage } from '@demicodes/plugin-sdk'
import { SettingsRow } from '@demicodes/plugin-sdk'
import type { AddSourceAnswer, SettingsSkill, SettingsSkillDraft, SettingsSkillSource } from './types'

/**
 * Skill sources (`skills.md` § The page): one git repository is a pack,
 * pinned to a commit, whose state is a status label after its name. Packs
 * start folded unless `open` names them. Opening a pack of more than six
 * skills scrolls inside the pack. Each skill is a row you can turn on, with
 * its own status labels; the SKILL.md files that are not skills follow, with
 * why. The pack's switch is on when any skill is on; flipping it sets every
 * skill in the pack. Removing a source asks first, naming its skills, which
 * go with it.
 */
const SKILL_LIST_CAP = 6

const props = defineProps<{
  sources: SettingsSkillSource[]
  overlayStore: OverlayStore
  /** The sources with a change being saved. */
  pending?: readonly string[]
  /** Adds a source; the dialog stays open until it is added. */
  addSource: (draft: SettingsSkillDraft) => Promise<AddSourceAnswer>
}>()

const emit = defineEmits<{
  remove: [source: string]
  update: [source: string]
  switch: [source: string, skill: string, enabled: boolean]
  switchSource: [source: string, enabled: boolean]
}>()

const addOpen = ref(false)
/** The packs shown open. */
const openIds = defineModel<string[]>('open', { default: () => [] })

/** The pack's name: its repository, the origin's last part. */
function name(source: SettingsSkillSource): string {
  const path = source.origin.replace(/\/+$/, '').replace(/\.git$/, '')
  return path.slice(path.lastIndexOf('/') + 1) || source.origin
}

/** The source's state: the fetch that runs, the last one's failure, or an update to fetch. */
function sourceStatuses(source: SettingsSkillSource): SettingsRowStatus[] {
  if (source.fetching) {
    // A source with no commit and no failure has never been fetched: its first fetch adds it.
    return [{ label: !source.commit && !source.failure ? 'Adding' : 'Updating' }]
  }
  if (source.failure) {
    return [{ label: 'Failed', tone: 'danger', detail: source.failure.message }]
  }
  if (source.updateAvailable) {
    return [{ label: 'Update available', tone: 'accent' }]
  }
  return []
}

/** What the skill's row says of it beside its switch. */
function skillStatuses(skill: SettingsSkill): SettingsRowStatus[] {
  const statuses: SettingsRowStatus[] = []
  if (skill.warnings.length) {
    statuses.push({ label: 'Warning', tone: 'warning', detail: skill.warnings.join('\n') })
  }
  if (skill.takenBy) {
    statuses.push({
      label: 'Name taken',
      detail: `A skill named "${skill.name}" from ${skill.takenBy} is on. Turn it off to turn this one on.`,
    })
  }
  if (skill.disableModelInvocation) {
    statuses.push({
      label: 'Never offered',
      detail: 'It sets disable-model-invocation, so the agent is never offered it.',
    })
  }
  return statuses
}

function pack(source: SettingsSkillSource) {
  const total = source.skills.length
  const on = source.skills.filter((skill) => skill.enabled).length
  return {
    total,
    on,
    checked: on > 0,
    // Mixed: "4/6". All on or all off: nothing beside the switch.
    label: on > 0 && on < total ? `${on}/${total}` : undefined,
  }
}

function busy(source: SettingsSkillSource): boolean {
  return props.pending?.includes(source.id) ?? false
}

function isOpen(id: string) {
  return openIds.value.includes(id)
}

function toggle(id: string) {
  openIds.value = isOpen(id)
    ? openIds.value.filter((entry) => entry !== id)
    : [...openIds.value, id]
}

function foldable(source: SettingsSkillSource): boolean {
  return source.skills.length + source.skipped.length > 0
}

const empty = computed(() => props.sources.length === 0)

/** The source whose removal is being asked about; it stays shown while the dialog closes. */
const removing = ref<SettingsSkillSource | null>(null)
const removeOpen = ref(false)
/** At most this many of the source's skills are named in the question; the rest are counted. */
const NAMED_SKILLS = 8

function askRemove(source: SettingsSkillSource): void {
  removing.value = source
  removeOpen.value = true
}

/** The skills that go with the source, as the question names them. */
const removedSkills = computed(() => {
  const names = (removing.value?.skills ?? []).map((skill) => skill.name)
  if (names.length <= NAMED_SKILLS) {
    return names
  }
  return [...names.slice(0, NAMED_SKILLS), `and ${names.length - NAMED_SKILLS} more`]
})

function confirmRemove(): void {
  if (removing.value) {
    emit('remove', removing.value.id)
  }
  removeOpen.value = false
}
</script>

<template>
  <SettingsPage
    title="Skills"
    description="Packaged workflows the agent can follow. Off keeps the files but hides them from the agent."
  >
    <SettingsGroup title="Sources">
      <template #actions>
        <Button size="sm" @click="addOpen = true">Add Source…</Button>
      </template>
      <div v-for="source in sources" :key="source.id">
        <SettingsRow
          :label="name(source)"
          :interactive="foldable(source)"
          :statuses="sourceStatuses(source)"
          :aria-expanded="foldable(source) ? isOpen(source.id) : undefined"
          @click="toggle(source.id)"
        >
          <template #leading>
            <GitBranch :size="ICON_PX.in28" />
          </template>
          <template #description>
            <span class="block truncate font-mono">{{ source.origin }}<template v-if="source.commit"> · {{ source.commit.slice(0, 7) }}</template></span>
          </template>
          <div class="flex items-center gap-2" @click.stop>
            <template v-if="pack(source).total">
              <span
                class="inline-grid h-4 grid-cols-1 grid-rows-1 items-center justify-items-end tabular-nums text-[12px] leading-4 text-fg-subtle"
              >
                <span
                  class="invisible col-start-1 row-start-1"
                  aria-hidden="true"
                >00/00</span>
                <span
                  class="col-start-1 row-start-1"
                  :class="pack(source).label ? '' : 'invisible'"
                >{{ pack(source).label || '00/00' }}</span>
              </span>
              <Switch
                :model-value="pack(source).checked"
                size="sm"
                :disabled="busy(source)"
                :aria-label="`Enable every skill in ${name(source)}`"
                @update:model-value="(on) => emit('switchSource', source.id, on)"
              />
            </template>
            <Tooltip content="Update">
              <IconButton
                size="sm"
                :icon="RefreshCw"
                spin-on-click
                :spinning="source.fetching"
                :disabled="source.fetching"
                aria-label="Update"
                @click="emit('update', source.id)"
              />
            </Tooltip>
            <Tooltip content="Remove">
              <IconButton
                size="sm"
                :icon="Trash2"
                variant="danger"
                aria-label="Remove"
                :disabled="busy(source)"
                @click="askRemove(source)"
              />
            </Tooltip>
          </div>
          <FoldChevron
            :open="isOpen(source.id)"
            :visible="foldable(source)"
            class="text-fg-subtle"
          />
        </SettingsRow>
        <Fold :open="isOpen(source.id) && foldable(source)">
          <ScrollArea
            v-if="foldable(source)"
            class="skill-list"
            :class="source.skills.length + source.skipped.length > SKILL_LIST_CAP ? 'skill-list-scroll' : ''"
            viewport-class="skill-list-rows"
          >
            <SettingsRow
              v-for="skill in source.skills"
              :key="skill.name"
              inset
              :label="skill.name"
              :muted="!skill.enabled"
              :statuses="skillStatuses(skill)"
            >
              <template #description>
                <span class="block truncate">{{ skill.description }}</span>
              </template>
              <Switch
                :model-value="skill.enabled"
                size="sm"
                :disabled="busy(source)"
                :aria-label="`${skill.name} ${skill.enabled ? 'on' : 'off'}`"
                @update:model-value="(on) => emit('switch', source.id, skill.name, on)"
              />
            </SettingsRow>
            <SettingsRow
              v-for="skipped in source.skipped"
              :key="skipped.path"
              inset
              muted
              :label="skipped.path"
            >
              <template #description>
                <span class="block truncate">Not a skill: {{ skipped.reason }}</span>
              </template>
            </SettingsRow>
          </ScrollArea>
        </Fold>
      </div>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        No Sources Yet
      </div>
    </SettingsGroup>
    <AddSkillSourceDialog
      :is-open="addOpen"
      :overlay-store="overlayStore"
      :add-source="addSource"
      @close="addOpen = false"
    />
    <ConfirmDialog
      :is-open="removeOpen"
      :overlay-store="overlayStore"
      :title="`Remove “${removing ? name(removing) : ''}”?`"
      action="Remove"
      :goes="removedSkills"
      @close="removeOpen = false"
      @confirm="confirmRemove"
    >
      <p>
        {{ removing?.skills.length
          ? 'Its skills go with it, and the agent is no longer offered them. You can add the repository again later.'
          : 'The source leaves your skills. You can add the repository again later.' }}
      </p>
    </ConfirmDialog>
  </SettingsPage>
</template>

<style scoped>
.skill-list {
  border-top: 1px solid var(--line-subtle);
}

.skill-list-rows > * + * {
  border-top: 1px solid var(--line-subtle);
}

/* Six inset rows (label + one-line description). More than six scrolls inside the pack. */
.skill-list-scroll {
  max-height: calc(6 * 3.125rem);
}

.skill-list-scroll :deep(.skill-list-rows) {
  overscroll-behavior: contain;
}
</style>
