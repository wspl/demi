<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { PairingPhase } from './pairing'
import Dialog from '../ui/Dialog.vue'
import Button from '../ui/Button.vue'
import CopyCode from '../ui/CopyCode.vue'
import TextInput from '../ui/TextInput.vue'
import InlineError from '../ui/InlineError.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import Segmented from '../ui/Segmented.vue'
import { deviceInstallCommand, deviceSystems, type DeviceInstallation, type DeviceSystem } from './installation'

const props = defineProps<{ isOpen: boolean; overlayStore: OverlayStore; installation: DeviceInstallation; phase: PairingPhase }>()
const emit = defineEmits<{ close: []; next: []; back: []; submit: [code: string] }>()
const code = ref('')
const system = ref<DeviceSystem>('linux')
const command = computed(() => deviceInstallCommand(props.installation, system.value))
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
        <SettingsRow label="System">
          <Segmented v-model="system" size="sm" :options="deviceSystems" aria-label="Device system" />
        </SettingsRow>
        <div class="flex flex-col gap-2 p-4">
          <p class="text-[12px] leading-4 text-fg-subtle">{{ system === 'windows' ? 'Run in PowerShell on the device. Keep it open.' : 'Run in a terminal on the device, locally or over SSH. Keep it open.' }}</p>
          <CopyCode :code="command" copy-label="Copy install command" />
        </div>
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
        <Button v-if="phase.kind === 'done'" variant="primary" @click="emit('close')">Done</Button>
        <template v-else>
          <Button v-if="phase.kind === 'code'" @click="emit('back')">Back</Button>
          <Button @click="emit('close')">Cancel</Button>
          <Button v-if="phase.kind === 'setup'" variant="primary" @click="emit('next')">Continue</Button>
          <Button v-else variant="primary" :disabled="phase.kind === 'pairing' || !code.trim()" @click="submit"><IndeterminateSpinner v-if="phase.kind === 'pairing'" :size="14" />{{ phase.kind === 'pairing' ? 'Pairing…' : 'Pair device' }}</Button>
        </template>
      </div>
    </div>
  </Dialog>
</template>
