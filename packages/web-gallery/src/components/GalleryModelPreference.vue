<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ThinkingConfig } from '@demicodes/core'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import {
  initialModelIntent,
  intentThinkingConfig,
} from '@demicodes/web-ui/agent/model-selection'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { demoModels, demoProviders } from '../fixtures/catalog'

// The gallery adapter substitutes an in-memory record for backend preferences.
const saved = ref(initialModelIntent({
  providerId: 'anthropic',
  modelId: 'claude-sonnet',
  thinkingEffort: 'high',
  serviceTierId: 'priority',
}))
const conversation = ref(initialModelIntent(saved.value))
const thinking = computed(() => intentThinkingConfig(conversation.value))
const generation = ref(1)

function remember(): void {
  saved.value = { ...conversation.value }
}

function selectModel(providerId: string, modelId: string): void {
  conversation.value = {
    providerId, modelId, thinkingEffort: null, serviceTierId: null,
  }
  remember()
}

function setThinking(config: ThinkingConfig): void {
  conversation.value.thinkingEffort = config.type === 'disabled'
    ? 'disabled'
    : config.type === 'effort' || config.type === 'adaptive' ? config.effort : null
  remember()
}

function setTier(tier: string | null): void {
  conversation.value.serviceTierId = tier
  remember()
}

function create(): void {
  conversation.value = initialModelIntent(saved.value)
  generation.value += 1
}
</script>

<template>
  <div class="flex flex-col items-start gap-3">
    <p class="text-sm text-fg-subtle">Conversation {{ generation }}</p>
    <ModelSelector
      :providers="demoProviders"
      :models="demoModels"
      :selected-provider-id="conversation.providerId"
      :selected-model-id="conversation.modelId"
      :thinking-config="thinking"
      :service-tier-id="conversation.serviceTierId"
      @select-model="selectModel"
      @change-thinking="setThinking"
      @change-service-tier="setTier"
    />
    <Button size="sm" @click="create">New conversation with saved choice</Button>
  </div>
</template>
