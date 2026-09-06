<script setup lang="ts">
import { computed, ref } from 'vue'
import SettingsProviders from '@demicodes/web-ui/settings/SettingsProviders.vue'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'
import { providers } from '../prototype/fixtures'

const resources = useResources()
const conversations = useConversations()
const message = ref('')

const rows = computed(() =>
  resources.providers.map((p) => ({ id: p.id, label: p.label, modelCount: p.models.length, isAvailable: p.isAvailable })),
)

function add(label: string) {
  const template = providers()[0]!
  resources.providers.push({ ...template, id: crypto.randomUUID(), label })
  message.value = 'Provider added.'
}

function test(id: string) {
  const provider = resources.providers.find((p) => p.id === id)
  message.value = provider?.isAvailable ? 'Connection successful.' : 'Connection failed.'
}

function setAvailable(id: string, available: boolean) {
  const provider = resources.providers.find((p) => p.id === id)
  if (provider) provider.isAvailable = available
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
  <SettingsProviders
    :providers="rows"
    :message="message"
    @test="test"
    @remove="remove"
    @set-available="setAvailable"
    @add="add"
  />
</template>
