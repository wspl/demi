<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsIntegrations from '@demicodes/web-ui/settings/SettingsIntegrations.vue'
import type { IntegrationChange, SettingsIntegration } from '@demicodes/web-ui/settings/integrations'
import { slow } from '../fixtures/slow'

/**
 * Settings › Integrations with a group chat's bot and a support desk. Every
 * control acts on the specimen's own integrations as the product's do:
 *
 * - Add Integration makes *Integration*, then *Integration 2*, and opens its
 *   page with its token shown this once.
 * - The name and the switches change the integration.
 * - Replace Token shows a new token in place of the hidden one.
 */
const all = {
  subagents: true,
  browser: true,
  web: true,
  skills: true,
  media: true,
  mobile: true,
  schedules: true,
  attachments: true,
}

const integrations = ref<SettingsIntegration[]>([
  {
    id: 'i1',
    name: 'Team Chat Bot',
    capabilities: { ...all, mobile: false, media: false },
    createdAt: '2026-10-02T09:12:00Z',
    clouds: 14,
    conversations: 238,
  },
  {
    id: 'i2',
    name: 'Support Desk',
    capabilities: { ...all, subagents: false, browser: false, mobile: false, schedules: false },
    createdAt: '2026-10-20T16:40:00Z',
    clouds: 1,
    conversations: 12,
  },
])
const tokens = ref<Record<string, string>>({})
const shown = ref<string | null>(null)
let made = 0

/** A token as the backend makes one: its prefix and 64 hexadecimal digits. */
function token(): string {
  const digits = Array.from({ length: 64 }, () => Math.floor(Math.random() * 16).toString(16)).join('')
  return `demi_int_${digits}`
}

/** The first free name, *Integration*, *Integration 2*, as the backend picks it. */
async function add(): Promise<void> {
  await slow()
  const names = new Set(integrations.value.map((integration) => integration.name))
  let name = 'Integration'
  for (let n = 2; names.has(name); n++)
    name = `Integration ${n}`
  made += 1
  const id = `new${made}`
  integrations.value = [
    ...integrations.value,
    { id, name, capabilities: { ...all }, createdAt: new Date().toISOString(), clouds: 0, conversations: 0 },
  ]
  tokens.value = { ...tokens.value, [id]: token() }
  shown.value = id
}

async function change(id: string, change: IntegrationChange): Promise<void> {
  await slow()
  integrations.value = integrations.value.map((integration) =>
    integration.id === id ? { ...integration, ...change } : integration,
  )
}

async function replaceToken(id: string): Promise<void> {
  await slow()
  tokens.value = { ...tokens.value, [id]: token() }
}
</script>

<template>
  <SettingsIntegrations
    :integrations="integrations"
    :tokens="tokens"
    :shown="shown"
    load="ready"
    :overlay-store="appOverlayStore"
    :on-add="add"
    :on-change="change"
    :on-replace-token="replaceToken"
    @retry="() => {}"
    @show="shown = $event"
  />
</template>
