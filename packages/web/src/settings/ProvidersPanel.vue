<script setup lang="ts">
import Button from '@demicodes/web-ui/ui/Button.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'

import { ref } from 'vue'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'
import { providers } from '../prototype/fixtures'

const resources = useResources()
const conversations = useConversations()
const label = ref('')
const message = ref('')

function add() {
  if (!label.value.trim()) return
  const provider = providers()[0]!
  resources.providers.push({ ...provider, id: crypto.randomUUID(), label: label.value.trim() })
  label.value = ''
  message.value = 'Demo provider added. Choose it in the message input.'
}

function remove(id: string) {
  if (conversations.items.some((c) => c.providerId === id && c.stream)) {
    message.value = 'Stop the running turn before removing this provider.'
    return
  }
  resources.providers = resources.providers.filter((p) => p.id !== id)
}
</script>

<template>
  <h3>Providers</h3>
  <p class="hint">Explore model selection with demo providers. No credentials are needed.</p>
  <div v-for="provider in resources.providers" :key="provider.id" class="provider-card">
    <div class="resource-row">
      <div class="resource-description">
        <TextInput v-model="provider.label" aria-label="Provider label" />
        <span>
          {{ provider.models.length }} models ·
          {{ provider.isAvailable ? 'Available' : 'Unavailable' }}
        </span>
      </div>
      <Button
        @click="
          message = provider.isAvailable
            ? 'Demo connection successful.'
            : 'Demo connection failed. Enable this provider to retry.'
        "
      >
        Test
      </Button>
      <Button @click="remove(provider.id)">Remove</Button>
    </div>
    <Checkbox v-model="provider.isAvailable" label="Available for conversations" />
  </div>
  <p v-if="!resources.providers.length" class="empty-note">
    Add a provider to choose a model and start chatting.
  </p>
  <form class="add-resource" @submit.prevent="add">
    <label>
      Provider label
      <TextInput v-model="label" placeholder="My provider" required maxlength="64" />
    </label>
    <Button @click="add" :disabled="!label.trim()">Add demo provider</Button>
  </form>
  <p v-if="message" role="status" class="hint">{{ message }}</p>
</template>
