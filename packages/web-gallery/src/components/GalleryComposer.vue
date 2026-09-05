<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import type { ThinkingConfig, TokenUsage } from '@demicodes/core'
import { ArrowUp, File as FileIcon, Plus, Square } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import AttachmentTile from '@demicodes/web-ui/agent/AttachmentTile.vue'
import { filePreviewUrl } from '@demicodes/web-ui/agent/message-input/attachments'
import ComposerShell from '@demicodes/web-ui/agent/ComposerShell.vue'
import ContextUsageIndicator from '@demicodes/web-ui/agent/ContextUsageIndicator.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { demoUsage } from '../fixtures/blocks'
import { demoModels, demoProviders } from '../fixtures/catalog'

const props = withDefaults(defineProps<{
  placeholder: string
  conversationId?: string
  running?: boolean
  compacting?: boolean
  draft?: string
  attachments?: { name: string; src?: string }[]
  focused?: boolean
  attachOpen?: boolean
  dropping?: boolean
  selectedProviderId?: string
  selectedModelId?: string
  serviceTierId?: string | null
  usage?: TokenUsage
}>(), {
  conversationId: 'demo',
  running: false,
  compacting: false,
  draft: '',
  attachments: () => [],
  focused: false,
  attachOpen: false,
  dropping: false,
  selectedProviderId: 'anthropic',
  selectedModelId: 'claude-sonnet',
  serviceTierId: null,
})

const emit = defineEmits<{
  send: [text: string]
  queue: [text: string]
  stop: []
  compact: []
}>()

const draft = ref(props.draft)
const focused = ref(false)
const startFocused = ref(props.focused)
const attached = ref(props.attachments.map((item) => ({ ...item })))
const fileInputRef = ref<HTMLInputElement>()
const providerId = ref(props.selectedProviderId)
const modelId = ref(props.selectedModelId)
const serviceTierId = ref<string | null>(props.serviceTierId ?? null)
const thinkingConfig = ref<ThinkingConfig>({ type: 'effort', effort: 'medium', summary: null })

const hasDraft = computed(() => draft.value.trim().length > 0 || attached.value.length > 0)
const expanded = computed(() => draft.value.includes('\n') || attached.value.length > 0)
const showFocused = computed(() => startFocused.value || focused.value)
const ringUsage = computed(() => props.usage ?? demoUsage)

function revokeOwned(item: { name: string; src?: string }): void {
  if (item.src?.startsWith('blob:')) URL.revokeObjectURL(item.src)
}

function takeDraft(): string | null {
  const text = draft.value.trim()
  const names = attached.value.map((item) => item.name)
  if (!text && names.length === 0) return null
  draft.value = ''
  for (const item of attached.value) revokeOwned(item)
  attached.value = []
  return text || names.join(', ')
}

function submit(): void {
  const text = takeDraft()
  if (!text) return
  if (props.running) emit('queue', text)
  else emit('send', text)
}

function pickFiles(): void {
  fileInputRef.value?.click()
}

function onFileChange(event: Event): void {
  const input = event.target as HTMLInputElement
  const files = input.files ? [...input.files] : []
  input.value = ''
  addDroppedFiles(files)
}

function removeAttachment(index: number): void {
  const item = attached.value[index]
  if (item) revokeOwned(item)
  attached.value = attached.value.filter((_, itemIndex) => itemIndex !== index)
}

function addDroppedFiles(files: File[]): void {
  if (files.length === 0) return
  attached.value = [
    ...attached.value,
    ...files.map((file) => ({
      name: file.name,
      src: filePreviewUrl(file),
    })),
  ]
}

onBeforeUnmount(() => {
  for (const item of attached.value) revokeOwned(item)
})

function selectModel(nextProviderId: string, nextModelId: string): void {
  providerId.value = nextProviderId
  modelId.value = nextModelId
}
</script>

<template>
  <div class="w-full">
    <input ref="fileInputRef" type="file" class="hidden" multiple @change="onFileChange" />
    <ComposerShell :focused="showFocused" :expanded="expanded" :dropping="dropping" @drop-files="addDroppedFiles">
      <template v-if="attached.length > 0" #chips>
        <AttachmentTile
          v-for="(item, index) in attached"
          :key="`${item.name}-${index}`"
          :name="item.name"
          :src="item.src"
          removable
          @remove="removeAttachment(index)"
        />
      </template>
      <template #editor>
        <textarea
          v-model="draft"
          rows="1"
          :placeholder="placeholder"
          class="w-full resize-none bg-transparent text-conversation text-fg outline-none placeholder:text-fg-subtle"
          @focus="focused = true"
          @blur="focused = false; startFocused = false"
          @keydown.meta.enter.prevent="submit"
          @keydown.ctrl.enter.prevent="submit"
        />
      </template>
      <template #attach>
        <Dropdown
          :overlay-store="appOverlayStore"
          :placement="attachOpen ? 'bottom-start' : 'top-start'"
          v-bind="attachOpen ? { open: true } : {}"
        >
          <template #trigger="{ isOpen }">
            <Tooltip content="Add attachment">
              <IconButton :icon="Plus" variant="ghost" circle :pressed="isOpen" />
            </Tooltip>
          </template>
          <template #content="{ close }">
            <Menu>
              <MenuItem :icon="FileIcon" label="Attach files" @select="close(); pickFiles()" />
            </Menu>
          </template>
        </Dropdown>
      </template>
      <template #model>
        <ModelSelector
          :providers="demoProviders"
          :models="demoModels"
          :selected-provider-id="providerId"
          :selected-model-id="modelId"
          :service-tier-id="serviceTierId"
          :thinking-config="thinkingConfig"
          @select-model="selectModel"
          @change-thinking="thinkingConfig = $event"
          @change-service-tier="serviceTierId = $event"
        />
      </template>
      <template #actions>
        <ContextUsageIndicator
          :conversation-id="conversationId"
          :usage="ringUsage"
          :context-window="200000"
          :input-limit="180000"
          :is-compacting="compacting"
          :is-clickable="!running"
          @compact="emit('compact')"
        />
        <Tooltip v-if="hasDraft" :content="running ? 'Queue next turn' : 'Send message'">
          <IconButton :icon="ArrowUp" variant="accent" circle @click="submit" />
        </Tooltip>
        <Tooltip v-else-if="running || compacting" content="Stop">
          <IconButton :icon="Square" variant="ghost" circle @click="emit('stop')" />
        </Tooltip>
        <IconButton v-else :icon="ArrowUp" variant="ghost" circle disabled />
      </template>
    </ComposerShell>
  </div>
</template>
