<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ThinkingConfig, TokenUsage } from '@demicodes/core'
import { ArrowUp, File as FileIcon, HardDrive, Plus, Square } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '../overlay/appOverlay'
import { t } from '../infra/i18n'
import AttachmentTile from './AttachmentTile.vue'
import {
  attachmentsReady,
  attachmentCaption,
  attachmentSendBlockReason,
  type ComposerAttachment,
} from './message-input/attachments'
import ComposerShell from './ComposerShell.vue'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import ModelSelector from './ModelSelector.vue'
import { composerModel } from './model-selection'
import SessionNoticeBar from './SessionNoticeBar.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Tooltip from '../ui/Tooltip.vue'

const props = withDefaults(
  defineProps<{
    placeholder: string
    conversationId?: string
    running?: boolean
    compacting?: boolean
    disabled?: boolean
    hasContent?: boolean
    multiline?: boolean
    canCompact?: boolean
    accept?: string
    attachments?: ComposerAttachment[]
    /** The conversation has a host with files: the menu offers a remote file beside local ones. */
    remoteFiles?: boolean
    focused?: boolean
    attachOpen?: boolean
    dropping?: boolean
    providers: ProviderInfo[]
    models: Record<string, ModelInfo[]>
    selectedProviderId?: string | null
    selectedModelId?: string | null
    thinkingConfig?: ThinkingConfig
    serviceTierId?: string | null
    usage?: TokenUsage | null
    /** When no model can send, show Configure models. Hide the action if the user cannot open that page. */
    canConfigure?: boolean
    /** Replaces the input with the archive bar. */
    archived?: boolean
  }>(),
  {
    attachments: () => [],
    canCompact: true,
    canConfigure: true,
  },
)
const draft = defineModel<string>('draft', { default: '' })
const emit = defineEmits<{
  submit: []
  stop: []
  compact: []
  addFiles: [files: File[]]
  /** Open the host's file browser; the caller attaches what it returns. */
  attachRemote: []
  removeAttachment: [id: string]
  retryAttachment: [id: string]
  selectModel: [providerId: string, modelId: string]
  changeThinking: [config: ThinkingConfig]
  changeServiceTier: [id: string | null]
  configure: []
  restore: []
}>()
const focused = ref(false)
const fileInput = ref<HTMLInputElement>()
const hasDraft = computed(
  () => props.hasContent || !!draft.value.trim() || !!props.attachments.length,
)
const modelState = computed(() =>
  composerModel(
    props.providers,
    props.models,
    props.selectedProviderId,
    props.selectedModelId,
  ),
)
const sendDisabled = computed(
  () =>
    props.disabled ||
    modelState.value.kind !== 'ready' ||
    !attachmentsReady(props.attachments),
)
const sendBlockReason = computed(() => {
  if (modelState.value.kind === 'unavailable') {
    return t('agent.input.switchModel')
  }
  if (props.disabled) {
    return undefined
  }
  return attachmentSendBlockReason(props.attachments)
})
const expanded = computed(
  () => props.multiline || draft.value.includes('\n') || !!props.attachments.length,
)
const selected = computed(() =>
  props.models[props.selectedProviderId ?? '']?.find(
    (model) => model.id === props.selectedModelId,
  ),
)

function submit() {
  if (!sendDisabled.value && hasDraft.value) {
    emit('submit')
  }
}

function pickFiles(close: () => void) {
  close()
  fileInput.value?.click()
}

function fileChange(event: Event) {
  const input = event.target as HTMLInputElement
  emit('addFiles', [...(input.files ?? [])])
  input.value = ''
}

function keydown(event: KeyboardEvent) {
  if (event.key !== 'Enter' || event.shiftKey || event.isComposing) {
    return
  }
  event.preventDefault()
  submit()
}
</script>

<template>
  <Transition
    name="composer-archive"
    mode="out-in"
  >
    <SessionNoticeBar
      v-if="archived"
      key="archived"
      :label="t('agent.session.archived')"
      :action="t('agent.session.restore')"
      @action="emit('restore')"
    />
    <SessionNoticeBar
      v-else-if="modelState.kind === 'none'"
      key="none"
      :label="t('agent.input.noModels')"
      :action="canConfigure !== false ? t('agent.input.configureModels') : undefined"
      @action="emit('configure')"
    />
    <div
      v-else
      key="composer"
      class="w-full"
    >
      <input
        ref="fileInput"
        type="file"
        class="hidden"
        multiple
        :accept="accept"
        @change="fileChange"
      />
      <ComposerShell
        :focused="focused || props.focused"
        :expanded="expanded"
        :dropping="dropping"
        @drop-files="emit('addFiles', $event)"
      >
        <template
          v-if="attachments.length"
          #chips
        >
          <Tooltip
            v-for="item in attachments"
            :key="item.id"
            :content="attachmentCaption(item)"
          >
            <AttachmentTile
              :name="item.name"
              :src="item.kind === 'file' ? item.src : undefined"
              :destination="item.kind === 'file' ? item.destination : undefined"
              :phase="item.kind === 'file' ? item.phase : undefined"
              :progress="item.kind === 'file' ? item.progress : undefined"
              removable
              @remove="emit('removeAttachment', item.id)"
              @retry="emit('retryAttachment', item.id)"
            />
          </Tooltip>
        </template>
        <template #editor>
          <slot name="editor">
            <textarea
              v-model="draft"
              rows="1"
              aria-label="Message"
              :placeholder="placeholder"
              class="w-full resize-none bg-transparent text-conversation text-fg outline-none placeholder:text-fg-subtle"
              @focus="focused = true"
              @blur="focused = false"
              @keydown="keydown"
            />
          </slot>
        </template>
        <template #attach>
          <Dropdown
            :overlay-store="appOverlayStore"
            :placement="attachOpen ? 'bottom-start' : 'top-start'"
            v-bind="attachOpen ? { open: true } : {}"
          >
            <template #trigger="{ isOpen }">
              <Tooltip :content="t('agent.input.attach')">
                <IconButton
                  :icon="Plus"
                  variant="ghost"
                  circle
                  :pressed="isOpen"
                  aria-label="Add attachment"
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  :icon="FileIcon"
                  :label="
                    t(
                      remoteFiles
                        ? 'agent.input.attachLocalFiles'
                        : 'agent.input.attachFiles',
                    )
                  "
                  @select="pickFiles(close)"
                />
                <MenuItem
                  v-if="remoteFiles"
                  :icon="HardDrive"
                  :label="t('agent.input.attachRemoteFile')"
                  @select="emit('attachRemote')"
                />
              </Menu>
            </template>
          </Dropdown>
        </template>
        <template #model>
          <ModelSelector
            :providers="providers"
            :models="models"
            :selected-provider-id="selectedProviderId"
            :selected-model-id="selectedModelId"
            :thinking-config="thinkingConfig"
            :service-tier-id="serviceTierId"
            @select-model="(provider, model) => emit('selectModel', provider, model)"
            @change-thinking="emit('changeThinking', $event)"
            @change-service-tier="emit('changeServiceTier', $event)"
          />
        </template>
        <template #actions>
          <ContextUsageIndicator
            :conversation-id="conversationId"
            :usage="usage"
            :context-window="selected?.contextWindow"
            :input-limit="selected?.inputLimit"
            :is-compacting="compacting"
            :is-clickable="!running && canCompact !== false"
            @compact="emit('compact')"
          />
          <Tooltip
            v-if="hasDraft"
            :content="running ? 'Queue next turn' : 'Send message'"
            :disabled="sendDisabled"
          >
            <IconButton
              :icon="ArrowUp"
              variant="accent"
              circle
              :disabled="sendDisabled"
              :disabled-reason="sendBlockReason"
              :aria-label="running ? 'Queue message' : 'Send message'"
              @click="submit"
            />
          </Tooltip>
          <Tooltip
            v-else-if="running || compacting"
            content="Stop"
          >
            <IconButton
              :icon="Square"
              variant="ghost"
              circle
              aria-label="Stop response"
              @click="emit('stop')"
            />
          </Tooltip>
          <IconButton
            v-else
            :icon="ArrowUp"
            variant="ghost"
            circle
            disabled
            aria-label="Send message"
          />
        </template>
      </ComposerShell>
    </div>
  </Transition>
</template>

<style scoped>
.composer-archive-enter-active,
.composer-archive-leave-active {
  transition:
    opacity 140ms ease,
    transform 140ms ease;
}

.composer-archive-enter-from,
.composer-archive-leave-to {
  opacity: 0;
  transform: translateY(6px);
}

@media (prefers-reduced-motion: reduce) {
  .composer-archive-enter-active,
  .composer-archive-leave-active {
    transition: none;
  }
}
</style>
