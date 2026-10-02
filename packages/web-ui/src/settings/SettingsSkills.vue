<script setup lang="ts">
import { computed, ref } from 'vue'
import { GitBranch, RefreshCw, Trash2 } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import AddSkillSourceDialog from './AddSkillSourceDialog.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsSkillDraft, SettingsSkillSource } from './types'

/**
 * Skill sources (`skills.md` § The page): one git repository is a pack,
 * pinned to a commit. Packs start folded. Opening a pack of more than six
 * skills scrolls inside the pack. Each skill is a row you can turn on; the
 * SKILL.md files that are not skills follow, with why. The pack's switch is
 * on when any skill is on; flipping it sets every skill in the pack.
 */
const SKILL_LIST_CAP = 6

const props = defineProps<{
  sources: SettingsSkillSource[]
  overlayStore: OverlayStore
  /** The sources with a change being saved. */
  pending?: readonly string[]
}>()

const emit = defineEmits<{
  add: [draft: SettingsSkillDraft]
  remove: [source: string]
  update: [source: string]
  switch: [source: string, skill: string, enabled: boolean]
  switchSource: [source: string, enabled: boolean]
}>()

const addOpen = ref(false)
const openIds = ref<string[]>([])

/** The pack's name: its repository, the origin's last part. */
function name(source: SettingsSkillSource): string {
  const path = source.origin.replace(/\/+$/, '').replace(/\.git$/, '')
  return path.slice(path.lastIndexOf('/') + 1) || source.origin
}

/** The fetch that runs, or the last one's failure, beside the name. */
function status(source: SettingsSkillSource): { word: string; dot: string; detail?: string } | null {
  if (source.fetching) {
    return { word: 'Updating', dot: 'bg-on-warning' }
  }
  if (source.failure) {
    return { word: 'Error', dot: 'bg-on-danger', detail: source.failure.message }
  }
  return null
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
</script>

<template>
  <SettingsPage
    title="Skills"
    description="Packaged workflows the agent can follow. Off keeps the files but hides them from the agent."
  >
    <SettingsGroup>
      <template #header>
        <header class="flex items-start justify-between gap-3">
          <div class="select-none">
            <h3 class="text-[15px] font-medium leading-5 text-fg-emphasis">Sources</h3>
          </div>
          <Button size="sm" @click="addOpen = true">Add source</Button>
        </header>
      </template>
      <div v-for="source in sources" :key="source.id">
        <SettingsRow
          :label="name(source)"
          :interactive="foldable(source)"
          :muted="!!source.failure && !source.fetching"
          :aria-expanded="foldable(source) ? isOpen(source.id) : undefined"
          @click="toggle(source.id)"
        >
          <template #leading>
            <GitBranch :size="ICON_PX.in28" />
          </template>
          <template #tags>
            <span
              class="inline-grid h-5 grid-cols-1 grid-rows-1 items-center text-[12px] leading-5 text-fg-muted"
            >
              <span
                class="invisible col-start-1 row-start-1 inline-flex items-center gap-1.5"
                aria-hidden="true"
              >
                <span class="size-1.5 rounded-full" />
                Updating
              </span>
              <span
                v-if="status(source)"
                class="col-start-1 row-start-1 inline-flex items-center gap-1.5"
              >
                <span
                  class="size-1.5 shrink-0 rounded-full"
                  :class="status(source)!.dot"
                />
                <Tooltip :content="status(source)!.detail" :disabled="!status(source)!.detail">
                  <span>{{ status(source)!.word }}</span>
                </Tooltip>
              </span>
            </span>
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
                @click="emit('remove', source.id)"
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
          <div
            v-if="foldable(source)"
            class="skill-list"
            :class="source.skills.length + source.skipped.length > SKILL_LIST_CAP ? 'skill-list-scroll' : ''"
          >
            <SettingsRow
              v-for="skill in source.skills"
              :key="skill.name"
              inset
              :label="skill.name"
              :muted="!skill.enabled"
            >
              <template v-if="skill.warnings.length || skill.disableModelInvocation" #tags>
                <Tooltip v-if="skill.warnings.length" :content="skill.warnings.join('\n')">
                  <span class="text-[11px] text-on-warning">Warning</span>
                </Tooltip>
                <span v-if="skill.disableModelInvocation" class="text-[11px] text-fg-subtle">Never offered to the agent</span>
              </template>
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
          </div>
        </Fold>
      </div>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        No sources yet.
      </div>
    </SettingsGroup>
    <AddSkillSourceDialog
      :is-open="addOpen"
      :overlay-store="overlayStore"
      @close="addOpen = false"
      @add="emit('add', $event)"
    />
  </SettingsPage>
</template>

<style scoped>
.skill-list {
  border-top: 1px solid var(--line-subtle);
}

.skill-list > * + * {
  border-top: 1px solid var(--line-subtle);
}

/* Six inset rows (label + one-line description). More than six scrolls inside the pack. */
.skill-list-scroll {
  max-height: calc(6 * 3.125rem);
  overflow-y: auto;
  overscroll-behavior: contain;
}
</style>
