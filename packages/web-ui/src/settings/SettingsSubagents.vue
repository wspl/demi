<script setup lang="ts">
import { computed, ref } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import Switch from '../ui/Switch.vue'
import Tag from '../ui/Tag.vue'
import TagLine from '../ui/TagLine.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SubagentProfileDialog from './SubagentProfileDialog.vue'
import { profileMissing, profileModelLabel } from './subagent-profiles'
import type { SettingsSubagentDraft, SettingsSubagentProfile } from './types'

/**
 * The Subagent section (`subagents.md` § Profiles): the switch that turns
 * subagents on or off for the user, and the user's profiles, each with a
 * switch of its own and an editor. A profile whose model settings name
 * something the catalog no longer offers is marked unavailable with the part
 * that is missing, which the page derives from the catalog it holds. The
 * host saves; the next spawn of every conversation uses a change.
 */
const props = defineProps<{
  /** The Subagent switch. */
  enabled: boolean
  profiles: SettingsSubagentProfile[]
  /** The providers and models the model menu offers. */
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  /**
   * Whether the catalog the marks are read from has loaded; until it has,
   * no profile is marked unavailable.
   */
  catalogReady: boolean
  overlayStore: OverlayStore
  /** Whether the Subagent switch is being saved. */
  switching?: boolean
  /** The profiles whose switch is being saved. */
  pending?: readonly string[]
  /** Creates a profile when `id` is null, else changes that one. */
  saveProfile: (id: string | null, draft: SettingsSubagentDraft) => Promise<void>
  deleteProfile: (id: string) => Promise<void>
}>()

const emit = defineEmits<{
  switch: [enabled: boolean]
  switchProfile: [id: string, enabled: boolean]
}>()

interface Editor {
  /** The profile edited; null for a new one. */
  id: string | null
  draft: SettingsSubagentDraft
  open: boolean
  status: { kind: 'idle' | 'saving' } | { kind: 'failed'; message: string }
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
  return props.catalogReady ? profileMissing(profile.model, props.providers, props.models) : null
}

/** What the row says of a profile besides its description. */
function traits(profile: SettingsSubagentProfile): string[] {
  return [
    profileModelLabel(profile.model, props.providers, props.models),
    profile.instructions === null ? 'Parent’s prompt' : 'Own prompt',
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
    status: { kind: 'idle' },
  }
}

function failed(error: unknown): Editor['status'] {
  return { kind: 'failed', message: error instanceof Error ? error.message : String(error) }
}

async function save(draft: SettingsSubagentDraft): Promise<void> {
  const current = editor.value
  if (!current || current.status.kind === 'saving') {
    return
  }
  current.draft = draft
  current.status = { kind: 'saving' }
  try {
    await props.saveProfile(current.id, draft)
    current.status = { kind: 'idle' }
    current.open = false
  } catch (error) {
    current.status = failed(error)
  }
}

/** Deleting asks first, over the profile's dialog. */
const deleteOpen = ref(false)

async function remove(): Promise<void> {
  deleteOpen.value = false
  const current = editor.value
  if (!current?.id || current.status.kind === 'saving') {
    return
  }
  current.status = { kind: 'saving' }
  try {
    await props.deleteProfile(current.id)
    current.status = { kind: 'idle' }
    current.open = false
  } catch (error) {
    current.status = failed(error)
  }
}

const empty = computed(() => props.profiles.length === 0)
</script>

<template>
  <SettingsPage
    title="Subagent"
    description="Child agents an agent starts for parts of its work. A change reaches the next spawn of every conversation, an open one included."
  >
    <SettingsGroup>
      <SettingsRow
        label="Subagents"
        :description="enabled
          ? 'Agents may start child agents.'
          : 'No agent starts a child agent. Children already running go on.'"
      >
        <Switch
          :model-value="enabled"
          size="sm"
          class="ml-1"
          :disabled="switching"
          :aria-label="`Subagents ${enabled ? 'on' : 'off'}`"
          @update:model-value="emit('switch', $event)"
        />
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
          :disabled="pending?.includes(profile.id)"
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
      :pending="editor.status.kind === 'saving'"
      :error="editor.status.kind === 'failed' ? editor.status.message : null"
      @close="editor.open = false"
      @save="save"
      @delete="deleteOpen = true"
    />
    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="`Delete the profile “${editor?.draft.name ?? ''}”?`"
      action="Delete"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>Agents can no longer spawn with it. Children already spawned with it go on as they are.</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
