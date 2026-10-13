<script setup lang="ts">
import { formError, type ShownError } from '../infra/errors'
import { computed, ref } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import type { ContextLimitChange } from '../agent/context-limit'
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
import type { SentenceText } from '../ui/ui-text'
import { answerInDialog } from './dialog-work'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SubagentProfileDialog from './SubagentProfileDialog.vue'
import { profileMissing, profileModelLabel } from './subagent-profiles'
import type { SettingsSubagentDraft, SettingsSubagentProfile } from './types'

/**
 * The Subagents section (`subagents.md` § Profiles, § Limits): the switch
 * that turns subagents on or off for the user, the two limits, each a
 * number or Unlimited after a warning, which keeps the number, and the
 * user's profiles, each with a switch of its own and an editor. A profile
 * whose model settings name something the catalog no longer offers is
 * marked unavailable with the part that is missing, which the page derives
 * from the catalog it holds. The host saves; the next spawn of every
 * conversation uses a change.
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
  /** The user's context limit on a model, set from a profile's model menu as from the composer's. */
  contextLimit: [change: ContextLimitChange]
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
    profile.instructions === null ? 'No instructions' : 'Own instructions',
    profile.canSpawn ? 'Children may spawn' : 'No spawning',
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
  return answerInDialog(() => shownEditor(id), 'Could Not Save the Profile', failed, () => props.saveProfile(id, draft))
}

/** Deleting asks first, over the profile's dialog. */
const deleteOpen = ref(false)

async function remove(): Promise<void> {
  const id = editor.value?.id
  if (!id) {
    return
  }
  await answerInDialog(() => shownEditor(id), 'Could Not Delete the Profile', failed, () => props.deleteProfile(id))
  // The question closes with its answer, if it still asks about this profile.
  if (editor.value?.id === id) {
    deleteOpen.value = false
  }
}

const empty = computed(() => props.profiles.length === 0)
</script>

<template>
  <SettingsPage
    title="Subagents"
    description="Child agents an agent starts for parts of its work. A change reaches the next spawn of every conversation, an open one included."
  >
    <SettingsGroup>
      <SettingsRow
        label="Subagents"
        description="Turned off, children already running go on."
      >
        <Switch
          :model-value="enabled"
          size="sm"
          class="ml-1"
          :aria-label="`Subagents ${enabled ? 'on' : 'off'}`"
          @update:model-value="emit('switch', $event)"
        />
      </SettingsRow>
      <SettingsRow
        label="Maximum subagents"
        description="The most subagents one conversation runs at once."
      >
        <template v-if="limitProblems.maxLive" #detail>
          <InlineError :message="limitProblems.maxLive" />
        </template>
        <LimitField
          :model-value="maxLive"
          label="Maximum subagents"
          :overlay-store="overlayStore"
          confirm-title="Allow unlimited subagents?"
          confirm-action="Allow Unlimited"
          @commit="emit('setLimit', 'maxLive', $event)"
          @problem="limitProblems.maxLive = $event"
        >
          <p>Agents may then start subagents without bound, which can make conversations unstable and spend many tokens.</p>
        </LimitField>
      </SettingsRow>
      <SettingsRow
        label="Maximum depth"
        description="At 1, only the main agent starts subagents."
      >
        <template v-if="limitProblems.maxDepth" #detail>
          <InlineError :message="limitProblems.maxDepth" />
        </template>
        <LimitField
          :model-value="maxDepth"
          label="Maximum depth"
          :overlay-store="overlayStore"
          confirm-title="Allow unlimited depth?"
          confirm-action="Allow Unlimited"
          @commit="emit('setLimit', 'maxDepth', $event)"
          @problem="limitProblems.maxDepth = $event"
        >
          <p>Subagents may then start subagents of their own without bound, which can make conversations unstable and spend many tokens.</p>
        </LimitField>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      title="Profiles"
      description="An agent names one when it spawns; without one, a child inherits its parent."
    >
      <template #actions>
        <Button size="sm" @click="open(null)">New Profile…</Button>
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
              Unavailable
            </Tag>
          </Tooltip>
        </template>
        <template #detail>
          <TagLine :items="traits(profile)" />
        </template>
        <Button size="sm" @click="open(profile)">Edit</Button>
        <Switch
          :model-value="profile.enabled"
          size="sm"
          class="ml-1"
          :aria-label="`${profile.name} ${profile.enabled ? 'on' : 'off'}`"
          @update:model-value="emit('switchProfile', profile.id, $event)"
        />
      </SettingsRow>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        No profiles. Every child inherits its parent’s model and prompt.
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
      @context-limit="emit('contextLimit', $event)"
    />
    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="`Delete the profile “${editor?.draft.name ?? ''}”?`"
      action="Delete"
      :operation="editing('delete', editor?.id ?? null)"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>Agents can no longer spawn with it. Children already spawned with it go on as they are.</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
