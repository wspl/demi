<script setup lang="ts">
import { ref, watch } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, Copy } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { PairingPhase } from './pairing'
import Dialog from '../ui/Dialog.vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import TextInput from '../ui/TextInput.vue'
import InlineError from '../ui/InlineError.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import SettingsRow from '../settings/SettingsRow.vue'

const props = defineProps<{ isOpen: boolean; overlayStore: OverlayStore; command: string; phase: PairingPhase }>()
const emit = defineEmits<{ close: []; next: []; back: []; submit: [code: string] }>()
const code = ref('')
const { copy, copied } = useClipboard()
watch(() => props.isOpen, () => { code.value = '' })
function submit() { if (props.phase.kind === 'code' && code.value.trim()) emit('submit', code.value.trim()) }
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" label="Add device" @close="emit('close')">
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add device</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">{{ phase.kind === 'setup' ? 'Start the runner on your device to get a pairing code.' : phase.kind === 'done' ? 'The device is linked to your account and ready to use.' : 'Enter the pairing code printed by the runner.' }}</p>
      </header>
      <div v-if="phase.kind === 'setup'" class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Run on your device" description="Use a local terminal or SSH. Keep the runner open.">
          <template #detail><code class="block break-words text-[12px] leading-5 text-fg-body">{{ command }}</code></template>
          <IconButton :icon="copied ? Check : Copy" size="sm" :aria-label="copied ? 'Copied' : 'Copy runner command'" @click="copy(command)" />
        </SettingsRow>
      </div>
      <template v-else-if="phase.kind === 'code' || phase.kind === 'pairing'">
        <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
          <SettingsRow label="Pairing code">
            <TextInput v-model="code" focused aria-label="Pairing code" placeholder="Paste code" class="w-60 max-w-full font-mono" :disabled="phase.kind === 'pairing'" @keydown.enter="submit" />
          </SettingsRow>
        </div>
        <InlineError v-if="phase.kind === 'code' && phase.error" :message="phase.error" />
      </template>
      <div v-else class="flex items-center gap-2 py-2 text-chrome text-fg">
        <Check :size="14" class="shrink-0 text-on-success" />
        <span class="min-w-0 truncate">{{ phase.device.name }}</span>
      </div>
      <div class="flex justify-end gap-2">
        <Button v-if="phase.kind === 'done'" size="sm" variant="primary" @click="emit('close')">Done</Button>
        <template v-else>
          <Button v-if="phase.kind === 'code'" size="sm" @click="emit('back')">Back</Button>
          <Button size="sm" @click="emit('close')">Cancel</Button>
          <Button v-if="phase.kind === 'setup'" size="sm" variant="primary" @click="emit('next')">Continue</Button>
          <Button v-else size="sm" variant="primary" :disabled="phase.kind === 'pairing' || !code.trim()" @click="submit"><IndeterminateSpinner v-if="phase.kind === 'pairing'" :size="14" />{{ phase.kind === 'pairing' ? 'Pairing…' : 'Pair device' }}</Button>
        </template>
      </div>
    </div>
  </Dialog>
</template>
