<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsMcpDraft, SettingsMcpTransport } from './types'

/**
 * Adding a tool server: a command the host runs, or a URL that is already listening,
 * and the name tools are prefixed with.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  close: []
  add: [draft: SettingsMcpDraft]
}>()

const transport = ref<SettingsMcpTransport>('stdio')
const target = ref('')
const name = ref('')

watch(() => props.isOpen, (open) => {
  if (!open) return
  transport.value = 'stdio'
  target.value = ''
  name.value = ''
})

const canAdd = computed(() => target.value.trim().length > 0 && name.value.trim().length > 0)

function submit() {
  if (!canAdd.value) return
  emit('add', { transport: transport.value, name: name.value.trim(), target: target.value.trim() })
  emit('close')
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" label="Add server" @close="emit('close')">
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add server</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">A command the host runs, or a URL that is already listening.</p>
      </header>

      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Transport">
          <Segmented
            size="sm"
            v-model="transport"
            :options="[{ value: 'stdio', label: 'Command' }, { value: 'http', label: 'URL' }]"
          />
        </SettingsRow>
        <SettingsRow
          :label="transport === 'stdio' ? 'Command' : 'URL'"
          :description="transport === 'stdio' ? 'The host starts this process and talks to it over stdio.' : undefined"
        >
          <TextInput
            v-model="target"
            focused
            class="w-72 max-w-full"
            :placeholder="transport === 'stdio' ? 'npx -y @modelcontextprotocol/server-memory' : 'https://mcp.example.com'"
            @keydown.enter="submit"
          />
        </SettingsRow>
        <SettingsRow label="Name" description="How tools show up: name_tool.">
          <TextInput v-model="name" placeholder="FileSystem" class="w-40 max-w-full" @keydown.enter="submit" />
        </SettingsRow>
      </div>

      <div class="flex justify-end gap-2">
        <Button @click="emit('close')">Cancel</Button>
        <Button variant="primary" :disabled="!canAdd" @click="submit">Add server</Button>
      </div>
    </div>
  </Dialog>
</template>
