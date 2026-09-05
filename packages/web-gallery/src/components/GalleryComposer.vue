<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import type { ThinkingConfig, TokenUsage } from '@demicodes/core'
import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import { filePreviewUrl } from '@demicodes/web-ui/agent/message-input/attachments'
import { demoUsage } from '../fixtures/blocks'
import { demoModels, demoProviders } from '../fixtures/catalog'

const props = withDefaults(
  defineProps<{
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
  }>(),
  {
    conversationId: 'demo',
    draft: '',
    attachments: () => [],
    selectedProviderId: 'anthropic',
    selectedModelId: 'claude-sonnet',
  },
)
const emit = defineEmits<{ send: [text: string]; queue: [text: string]; stop: []; compact: [] }>()
const draft = ref(props.draft)
const attached = ref(props.attachments.map((item) => ({ ...item })))
const providerId = ref(props.selectedProviderId)
const modelId = ref(props.selectedModelId)
const tier = ref(props.serviceTierId ?? null)
const thinking = ref<ThinkingConfig>({ type: 'effort', effort: 'medium', summary: null })

function remove(index: number) {
  const item = attached.value[index]
  if (item?.src?.startsWith('blob:')) URL.revokeObjectURL(item.src)
  attached.value.splice(index, 1)
}
function submit() {
  const text = draft.value.trim() || attached.value.map((item) => item.name).join(', ')
  if (!text) return
  draft.value = ''
  while (attached.value.length) remove(0)
  if (props.running) emit('queue', text)
  else emit('send', text)
}
function addFiles(files: File[]) {
  attached.value.push(...files.map((file) => ({ name: file.name, src: filePreviewUrl(file) })))
}
function selectModel(provider: string, model: string) {
  providerId.value = provider
  modelId.value = model
}
onBeforeUnmount(() => {
  while (attached.value.length) remove(0)
})
</script>

<template>
  <SessionComposer
    v-bind="props"
    v-model:draft="draft"
    :attachments="attached"
    :providers="demoProviders"
    :models="demoModels"
    :selected-provider-id="providerId"
    :selected-model-id="modelId"
    :service-tier-id="tier"
    :thinking-config="thinking"
    :usage="props.usage ?? demoUsage"
    @submit="submit"
    @add-files="addFiles"
    @remove-attachment="remove"
    @select-model="selectModel"
    @change-thinking="thinking = $event"
    @change-service-tier="tier = $event"
    @stop="emit('stop')"
    @compact="emit('compact')"
  />
</template>
