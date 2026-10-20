<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref } from 'vue'
import { GitBranch, RefreshCw, Trash2 } from '@lucide/vue'
import type { Action, OverlayStore, PageContext, SettingsRowStatus } from '@demicodes/plugin-sdk'
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
 * Skill sources: one git repository is a pack, pinned to a commit, whose
 * state is a status label after its name. Packs start folded unless `open`
 * names them. Opening a pack of more than six skills scrolls inside the
 * pack. Each skill is a row you can turn on, with its own status labels; the
 * SKILL.md files that are not skills follow, with why. The pack's switch is
 * on when any skill is on; flipping it sets every skill in the pack.
 * Removing a source asks first, naming its skills, which go with it. Each
 * control's call is an operation of the page, named by the plugin's method
 * and what it acts on: the control shows it working whenever it is drawn and
 * takes no input meanwhile.
 */
const SKILL_LIST_CAP = 6

const props = defineProps<{
  sources: SettingsSkillSource[]
  overlayStore: OverlayStore
  /** The page's operation of a method on what a key names (`PageContext.operation`). */
  operation: PageContext['operation']
  /** Adds a source; the dialog stays open until it is added. */
  addSource: (draft: SettingsSkillDraft) => Promise<AddSourceAnswer>
  /** Removes a source; Remove in the question works until it settles. */
  onRemove?: Action<[source: string]>
  /** Fetches a source again; its button works until it settles. */
  onUpdate?: Action<[source: string]>
  /** Turns one skill on or off; its switch works until it settles. */
  onSwitch?: Action<[source: string, skill: string, enabled: boolean]>
  /** Turns every skill of a source on or off; the source's switch works until it settles. */
  onSwitchSource?: Action<[source: string, enabled: boolean]>
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
    return [{ label: !source.commit && !source.failure ? t('sources.adding') : t('sources.updating') }]
  }
  if (source.failure) {
    return [{ label: t('sources.failed'), tone: 'danger', detail: source.failure.message }]
  }
  if (source.updateAvailable) {
    return [{ label: t('sources.updateAvailable'), tone: 'accent' }]
  }
  return []
}

/** What the skill's row says of it beside its switch. */
function skillStatuses(skill: SettingsSkill): SettingsRowStatus[] {
  const statuses: SettingsRowStatus[] = []
  if (skill.warnings.length) {
    statuses.push({ label: t('skill.warning'), tone: 'warning', detail: skill.warnings.join('\n') })
  }
  if (skill.takenBy) {
    statuses.push({
      label: t('skill.nameTaken'),
      detail: t('skill.nameTakenDetail', { name: skill.name, origin: skill.takenBy }),
    })
  }
  if (skill.disableModelInvocation) {
    statuses.push({
      label: t('skill.neverOffered'),
      detail: t('skill.neverOfferedDetail'),
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

/** The skill's key among the operations: its source and its name. */
function skillKey(source: SettingsSkillSource, skill: SettingsSkill): string {
  return `${source.id}/${skill.name}`
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
  return [...names.slice(0, NAMED_SKILLS), t('removeSource.more', { count: names.length - NAMED_SKILLS })]
})

/** Remove confirmed: once it is done, the question closes, if it still asks about this source. */
async function confirmRemove(): Promise<void> {
  const source = removing.value
  if (!source) {
    return
  }
  try {
    await props.onRemove?.(source.id)
  } finally {
    if (removing.value?.id === source.id) {
      removeOpen.value = false
    }
  }
}
</script>

<template>
  <SettingsPage
    :title="t('title')"
    :description="t('description')"
  >
    <SettingsGroup :title="t('sources.title')">
      <template #actions>
        <Button size="sm" @click="addOpen = true">{{ t('sources.add') }}</Button>
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
                :operation="operation('set_source_enabled', source.id)"
                :aria-label="t('sources.enableAll', { name: name(source) })"
                @update:model-value="(on) => onSwitchSource?.(source.id, on)"
              />
            </template>
            <Tooltip :content="t('sources.update')">
              <IconButton
                size="sm"
                :icon="RefreshCw"
                spin-on-click
                :spinning="source.fetching"
                :disabled="source.fetching"
                :operation="operation('update_source', source.id)"
                :aria-label="t('sources.update')"
                @click="onUpdate?.(source.id)"
              />
            </Tooltip>
            <Tooltip :content="t('sources.remove')">
              <IconButton
                size="sm"
                :icon="Trash2"
                variant="danger"
                :aria-label="t('sources.remove')"
                :operation="operation('remove_source', source.id)"
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
                :operation="operation('set_enabled', skillKey(source, skill))"
                :aria-label="skill.name"
                @update:model-value="(on) => onSwitch?.(source.id, skill.name, on)"
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
                <span class="block truncate">{{ t('skill.notASkill', { reason: skipped.reason }) }}</span>
              </template>
            </SettingsRow>
          </ScrollArea>
        </Fold>
      </div>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        {{ t('sources.none') }}
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
      :title="t('removeSource.title', { name: removing ? name(removing) : '' })"
      :action="t('sources.remove')"
      :goes="removedSkills"
      :operation="removing ? operation('remove_source', removing.id) : undefined"
      @close="removeOpen = false"
      @confirm="confirmRemove"
    >
      <p>
        {{ removing?.skills.length
          ? t('removeSource.withSkills')
          : t('removeSource.empty') }}
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
