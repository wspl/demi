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
import type {
  SettingsSkillDraft,
  SettingsSkillSource,
  SettingsSkillSourceState
} from './types'

/**
 * Skill sources: one git repository is a pack. Packs start folded. Opening a
 * pack of more than six skills scrolls inside the pack. Each SKILL.md is a
 * row you can turn on. The pack's switch is on when any skill is on;
 * flipping it sets every skill in the pack.
 */
const SKILL_LIST_CAP = 6

const props = defineProps<{
  sources: SettingsSkillSource[]
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  add: [draft: SettingsSkillDraft]
  remove: [source: SettingsSkillSource]
  update: [source: SettingsSkillSource]
}>()

const addOpen = ref(false)
const openIds = ref<string[]>([])

const statusWord: Partial<Record<SettingsSkillSourceState, string>> = {
  updating: 'Updating',
  error: 'Error',
}

const statusDot: Partial<Record<SettingsSkillSourceState, string>> = {
  updating: 'bg-on-warning',
  error: 'bg-on-danger',
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

function isOpen(id: string) {
  return openIds.value.includes(id)
}

function toggle(id: string) {
  openIds.value = isOpen(id)
    ? openIds.value.filter((entry) => entry !== id)
    : [...openIds.value, id]
}

function setAll(source: SettingsSkillSource, on: boolean) {
  for (const skill of source.skills) skill.enabled = on
}

function add(draft: SettingsSkillDraft) {
  emit('add', draft)
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
          :label="source.name"
          :interactive="pack(source).total > 0"
          :muted="source.state === 'error'"
          :aria-expanded="pack(source).total > 0 ? isOpen(source.id) : undefined"
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
                v-if="source.state !== 'ready'"
                class="col-start-1 row-start-1 inline-flex items-center gap-1.5"
              >
                <span
                  class="size-1.5 shrink-0 rounded-full"
                  :class="statusDot[source.state]"
                />
                <Tooltip :content="source.detail" :disabled="!source.detail">
                  <span>{{ statusWord[source.state] }}</span>
                </Tooltip>
              </span>
            </span>
          </template>
          <template #description>
            <span class="block truncate font-mono">{{ source.origin }}</span>
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
                :aria-label="`Enable every skill in ${source.name}`"
                @update:model-value="(on) => setAll(source, on)"
              />
            </template>
            <Tooltip content="Update from git">
              <IconButton
                size="sm"
                :icon="RefreshCw"
                spin-on-click
                :spinning="source.state === 'updating'"
                :disabled="source.state === 'updating'"
                aria-label="Update from git"
                @click="emit('update', source)"
              />
            </Tooltip>
            <Tooltip content="Remove source">
              <IconButton
                size="sm"
                :icon="Trash2"
                variant="danger"
                aria-label="Remove source"
                @click="emit('remove', source)"
              />
            </Tooltip>
          </div>
          <FoldChevron
            :open="isOpen(source.id)"
            :visible="pack(source).total > 0"
            class="text-fg-subtle"
          />
        </SettingsRow>
        <Fold :open="isOpen(source.id) && source.skills.length > 0">
          <div
            v-if="source.skills.length"
            class="skill-list"
            :class="source.skills.length > SKILL_LIST_CAP ? 'skill-list-scroll' : ''"
          >
            <SettingsRow
              v-for="skill in source.skills"
              :key="skill.id"
              inset
              :label="skill.name"
              :class="skill.enabled ? '' : 'opacity-60'"
            >
              <template #description>
                <span class="block truncate">{{ skill.description }}</span>
              </template>
              <Switch v-model="skill.enabled" size="sm" />
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
      @add="add"
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
