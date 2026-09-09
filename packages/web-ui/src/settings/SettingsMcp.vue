<script setup lang="ts">
import { AppWindow, RotateCw, Server } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import TagLine from '@demicodes/web-ui/ui/TagLine.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { ref } from 'vue'
import AddMcpServerDialog from './AddMcpServerDialog.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsMcpDraft, SettingsMcpServer, SettingsMcpState } from './types'

/**
 * MCP servers: one row each. Status is a dot and a word; errors live on that
 * status. Tools are a one-line tag row. Adding is a dialog.
 */
defineProps<{
  servers: SettingsMcpServer[]
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  add: [draft: SettingsMcpDraft]
  signIn: [server: SettingsMcpServer]
  restart: [server: SettingsMcpServer]
}>()

const addOpen = ref(false)
const restarting = ref(new Set<string>())

function restart(server: SettingsMcpServer) {
  restarting.value.add(server.id)
  emit('restart', server)
}

const statusWord: Record<SettingsMcpState, string> = {
  connected: 'Connected',
  auth: 'Sign in',
  crashed: 'Crashed',
  disabled: 'Off',
}

const statusDot: Record<SettingsMcpState, string> = {
  connected: 'bg-on-success',
  auth: 'bg-on-warning',
  crashed: 'bg-on-danger',
  disabled: 'bg-fg-ghost',
}

function statusOf(server: SettingsMcpServer): SettingsMcpState {
  return server.enabled ? server.state : 'disabled'
}

function add(draft: SettingsMcpDraft) {
  emit('add', draft)
}
</script>

<template>
  <SettingsPage
    title="MCP servers"
    description="Tool servers the agent can call. Off keeps the config but hides the tools."
  >
    <SettingsGroup>
      <template #header>
        <header class="flex items-start justify-between gap-3">
          <div class="select-none">
            <h3 class="text-[15px] font-medium leading-5 text-fg-emphasis">Servers</h3>
          </div>
          <Button size="sm" @click="addOpen = true">Add server</Button>
        </header>
      </template>
      <SettingsRow
        v-for="server in servers"
        :key="server.id"
        :label="server.name"
        :class="server.enabled ? '' : 'opacity-60'"
      >
        <template #leading>
          <component
            :is="server.transport === 'stdio' ? AppWindow : Server"
            :size="ICON_PX.in28"
          />
        </template>
        <template #tags>
          <Tooltip :content="server.detail" :disabled="!server.detail">
            <span
              class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted"
            >
              <span
                class="size-1.5 shrink-0 rounded-full"
                :class="statusDot[statusOf(server)]"
              />
              {{ statusWord[statusOf(server)] }}
            </span>
          </Tooltip>
        </template>
        <template #description>
          <Tooltip tag="div" :content="server.target">
            <span class="block truncate font-mono">{{ server.target }}</span>
          </Tooltip>
        </template>
        <template v-if="server.tools.length" #detail>
          <TagLine :items="server.tools" />
        </template>
        <Button
          v-if="server.enabled && server.state === 'auth'"
          size="sm"
          @click="emit('signIn', server)"
        >Sign in</Button>
        <Button
          v-else-if="server.enabled && (server.state === 'crashed' || restarting.has(server.id))"
          size="sm"
          spin-on-click
          :disabled="restarting.has(server.id)"
          @click="restart(server)"
          @spin-end="restarting.delete(server.id)"
        >
          <RotateCw :size="ICON_PX.in24" />
          Restart
        </Button>
        <Switch
          v-model="server.enabled"
          size="sm"
          class="ml-1"
        />
      </SettingsRow>
      <div
        v-if="!servers.length"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        No servers yet.
      </div>
    </SettingsGroup>
    <AddMcpServerDialog
      :is-open="addOpen"
      :overlay-store="overlayStore"
      @close="addOpen = false"
      @add="add"
    />
  </SettingsPage>
</template>
