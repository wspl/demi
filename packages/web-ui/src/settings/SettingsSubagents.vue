<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { formError, type ShownError } from '../infra/errors'
import { computed, ref } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import InlineError from '../ui/InlineError.vue'
import LimitField from '../ui/LimitField.vue'
import type { LimitValue } from '../ui/limit-field'
import Switch from '../ui/Switch.vue'
import Tag from '../ui/Tag.vue'
import TagLine from '../ui/TagLine.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { formatTokens } from '../ui/token-count'
import type { SentenceText } from '../ui/ui-text'
import { answerInDialog } from './dialog-work'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SubagentProfileDialog from './SubagentProfileDialog.vue'
import { profileMissing, profileModelLabel } from './subagent-profiles'
import type { SettingsSubagentDraft, SettingsSubagentProfile } from './types'

/**
 * The Subagents section (`crates/agent-server/src/server/tree/profiles.rs`).
 *
 * - The switch that turns subagents on or off for the user.
 * - The two limits, each a number or Unlimited after a warning, which keeps the
 *   number.
 * - The user's profiles, each with a switch of its own and an editor. A profile
 *   whose model settings name something the catalog no longer offers is marked
 *   unavailable with the part that is missing, which the page derives from the
 *   catalog it holds.
 *
 * The host saves; the next spawn of every conversation uses a change.
 */
const props = defineProps<{
  /** The Subagents switch. */
  enabled: boolean
  /** Maximum subagents: how many of a conversation are live at once. */
  maxLive: LimitValue
  /** Maximum depth: how many levels below the main agent. */
  maxDepth: LimitValue
  profiles: SettingsSubagentProfile[]
  /** The providers and models the model menu offers, as the composer's does. */
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  /**
   * Every model each provider's catalog lists, the ones the menu hides
   * included: a profile's model is named, and marked as gone, from these.
   */
  catalogModels: Record<string, ModelInfo[]>
  /**
   * Whether the catalog the marks are read from has loaded; until it has,
   * no profile is marked unavailable.
   */
  catalogReady: boolean
  overlayStore: OverlayStore
  /** Creates a profile when `id` is null, else changes that one. */
  saveProfile: (id: string | null, draft: SettingsSubagentDraft) => Promise<void>
  deleteProfile: (id: string) => Promise<void>
}>()

const emit = defineEmits<{
  switch: [enabled: boolean]
  /** A new limit; Unlimited only once the page confirmed it. */
  setLimit: [limit: 'maxLive' | 'maxDepth', value: LimitValue]
  switchProfile: [id: string, enabled: boolean]
}>()

/** What each limit's field says of text that is no limit. */
const limitProblems = ref<Record<'maxLive' | 'maxDepth', SentenceText | null>>({ maxLive: null, maxDepth: null })

interface Editor {
  /** The profile edited; null for a new one. */
  id: string | null
  draft: SettingsSubagentDraft
  open: boolean
  /** Why the last save or delete was refused, shown in the dialog. */
  error: ShownError | null
}

const editor = ref<Editor | null>(null)

const NEW_PROFILE: SettingsSubagentDraft = {
  name: '',
  description: '',
  model: null,
  contextLimit: null,
  instructions: null,
  canSpawn: true,
}

function missing(profile: SettingsSubagentProfile): string | null {
  return props.catalogReady ? profileMissing(profile.model, props.providers, props.catalogModels) : null
}

/** What the row says of a profile besides its description. */
function traits(profile: SettingsSubagentProfile): string[] {
  return [
    profileModelLabel(profile.model, props.providers, props.catalogModels),
    ...(profile.contextLimit === null ? [] : [t('subagents.context', { size: formatTokens(profile.contextLimit) })]),
    profile.instructions === null ? t('subagents.noInstructions') : t('subagents.ownInstructions'),
    profile.canSpawn ? t('subagents.mayStart') : t('subagents.noSpawning'),
  ]
}

function open(profile: SettingsSubagentProfile | null): void {
  editor.value = {
    id: profile?.id ?? null,
    draft: profile
      ? {
          name: profile.name,
          description: profile.description,
          model: profile.model,
          contextLimit: profile.contextLimit,
          instructions: profile.instructions,
          canSpawn: profile.canSpawn,
        }
      : { ...NEW_PROFILE },
    open: true,
    error: null,
  }
}

function failed(error: unknown): ShownError {
  // A refused name or field is the user's to correct in the dialog.
  return formError(error, { invalid_body: null, profile_exists: null })
}

/** The profile as the target of its Save and Delete, one group of operations (`operations.ts`). */
function profileTarget(id: string | null): string {
  return `profile:${id ?? 'new'}`
}

/** Save and Delete exclude each other on one profile: while one works, the other cannot be pressed. */
function editing(action: 'save' | 'delete', id: string | null) {
  return { action, target: profileTarget(id), group: 'edit' }
}

/** The dialog of profile `id`, if it is the one open: the answer to its work goes there and nowhere else. */
function shownEditor(id: string | null): Editor | null {
  const current = editor.value
  return current?.open && current.id === id ? current : null
}

function save(draft: SettingsSubagentDraft): Promise<void> {
  const current = editor.value
  if (!current) {
    return Promise.resolve()
  }
  // The draft stays, for the dialog to keep its fields until the backend took them.
  current.draft = draft
  const id = current.id
  return answerInDialog(() => shownEditor(id), t('subagents.saveFailed'), failed, () => props.saveProfile(id, draft))
}

/** Deleting asks first, over the profile's dialog. */
const deleteOpen = ref(false)

async function remove(): Promise<void> {
  const id = editor.value?.id
  if (!id) {
    return
  }
  await answerInDialog(() => shownEditor(id), t('subagents.deleteFailed'), failed, () => props.deleteProfile(id))
  // The question closes with its answer, if it still asks about this profile.
  if (editor.value?.id === id) {
    deleteOpen.value = false
  }
}

const empty = computed(() => props.profiles.length === 0)
</script>

<template>
  <SettingsPage
    :title="t('subagents.title')"
    :description="t('subagents.description')"
  >
    <SettingsGroup>
      <SettingsRow
        :label="t('subagents.title')"
        :description="t('subagents.switchDescription')"
      >
        <Switch
          :model-value="enabled"
          size="sm"
          class="ml-1"
          :aria-label="t('subagents.title')"
          @update:model-value="emit('switch', $event)"
        />
      </SettingsRow>
      <SettingsRow
        :label="t('subagents.maxLive')"
        :description="t('subagents.maxLiveDescription')"
      >
        <template v-if="limitProblems.maxLive" #detail>
          <InlineError :message="limitProblems.maxLive" />
        </template>
        <LimitField
          :model-value="maxLive"
          :label="t('subagents.maxLive')"
          :overlay-store="overlayStore"
          :confirm-title="t('subagents.maxLiveConfirmTitle')"
          :confirm-action="t('subagents.allowUnlimited')"
          @commit="emit('setLimit', 'maxLive', $event)"
          @problem="limitProblems.maxLive = $event"
        >
          <p>{{ t('subagents.unlimitedWarning') }}</p>
        </LimitField>
      </SettingsRow>
      <SettingsRow
        :label="t('subagents.maxDepth')"
        :description="t('subagents.maxDepthDescription')"
      >
        <template v-if="limitProblems.maxDepth" #detail>
          <InlineError :message="limitProblems.maxDepth" />
        </template>
        <LimitField
          :model-value="maxDepth"
          :label="t('subagents.maxDepth')"
          :overlay-store="overlayStore"
          :confirm-title="t('subagents.maxDepthConfirmTitle')"
          :confirm-action="t('subagents.allowUnlimited')"
          @commit="emit('setLimit', 'maxDepth', $event)"
          @problem="limitProblems.maxDepth = $event"
        >
          <p>{{ t('subagents.unlimitedWarning') }}</p>
        </LimitField>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      :title="t('subagents.profiles')"
      :description="t('subagents.profilesDescription')"
    >
      <template #actions>
        <Button size="sm" @click="open(null)">{{ t('subagents.newProfile') }}</Button>
      </template>
      <SettingsRow
        v-for="profile in profiles"
        :key="profile.id"
        :label="profile.name"
        :description="profile.description"
        :muted="!profile.enabled || !enabled"
        :target="profileTarget(profile.id)"
        interactive
        isolate-controls
        @click="open(profile)"
      >
        <template v-if="missing(profile)" #tags>
          <Tooltip :content="missing(profile) ?? ''">
            <Tag tone="warning">
              <TriangleAlert :size="ICON_PX.in20" class="mr-1" />
              {{ t('subagents.unavailable') }}
            </Tag>
          </Tooltip>
        </template>
        <template #detail>
          <TagLine :items="traits(profile)" />
        </template>
        <Button size="sm" @click="open(profile)">{{ t('subagents.edit') }}</Button>
        <Switch
          :model-value="profile.enabled"
          size="sm"
          class="ml-1"
          :aria-label="profile.name"
          @update:model-value="emit('switchProfile', profile.id, $event)"
        />
      </SettingsRow>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        {{ t('subagents.none') }}
      </div>
    </SettingsGroup>

    <SubagentProfileDialog
      v-if="editor"
      :is-open="editor.open"
      :overlay-store="overlayStore"
      :mode="editor.id === null ? 'create' : 'edit'"
      :profile="editor.draft"
      :providers="providers"
      :models="models"
      :catalog-models="catalogModels"
      :error="editor.error"
      :save-operation="editing('save', editor.id)"
      :delete-operation="editing('delete', editor.id)"
      @close="editor.open = false"
      @save="save"
      @delete="deleteOpen = true"
    />
    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="t('subagents.deleteTitle', { name: editor?.draft.name ?? '' })"
      :action="uiText('common.delete')"
      :operation="editing('delete', editor?.id ?? null)"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>{{ t('subagents.deleteBody') }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
