<script setup lang="ts">
import { ref } from 'vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import {
  applyModelChange,
  initialModelSettings,
  type ModelSettingsChange,
} from '@demicodes/web-ui/agent/model-selection'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { demoModels, demoProviders } from '../fixtures/catalog'

// The gallery adapter substitutes an in-memory record for backend preferences.
const saved = ref(initialModelSettings({
  providerId: 'anthropic',
  modelId: 'claude-sonnet',
  thinkingEffort: 'high',
  serviceTierId: 'priority',
}))
const conversation = ref(initialModelSettings(saved.value))
const generation = ref(1)

function change(next: ModelSettingsChange): void {
  conversation.value = applyModelChange(conversation.value, next)
  saved.value = { ...conversation.value }
}

function create(): void {
  conversation.value = initialModelSettings(saved.value)
  generation.value += 1
}
</script>

<template>
  <div class="flex flex-col items-start gap-3">
    <p class="text-sm text-fg-subtle">Conversation {{ generation }}</p>
    <ModelSelector
      :providers="demoProviders"
      :models="demoModels"
      :settings="conversation"
      @change="change"
    />
    <Button size="sm" @click="create">New conversation with saved choice</Button>
  </div>
</template>
