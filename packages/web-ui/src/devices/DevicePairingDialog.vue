<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import { clientPlatform } from '@demicodes/utils'
import { Check } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { PairingPhase } from './pairing'
import Dialog from '../ui/Dialog.vue'
import Button from '../ui/Button.vue'
import CopyCode from '../ui/CopyCode.vue'
import TextInput from '../ui/TextInput.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import InlineError from '../ui/InlineError.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import Segmented from '../ui/Segmented.vue'
import {
  deviceSystemOf,
  deviceSystems,
  type DeviceInstallCommands,
  type DeviceSystem,
} from './installation'

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  installCommands: DeviceInstallCommands
  phase: PairingPhase /** Stack on an open dialog, when opened from inside one. */
  stack?: boolean
}>()
const emit = defineEmits<{
  close: []
  next: []
  back: []
  submit: [code: string]
}>()
const code = ref('')
const system = ref<DeviceSystem>(deviceSystemOf(clientPlatform(navigator)))
const command = computed(() => props.installCommands[system.value])
watch(
  () => props.isOpen,
  () => {
    code.value = ''
  },
)
function submit() {
  if (props.phase.kind === 'code' && code.value.trim())
    emit('submit', code.value.trim())
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="t('pairing.title')"
    :stack="stack"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('pairing.title') }}</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          {{
            phase.kind === 'setup'
              ? t('pairing.setupHint')
              : phase.kind === 'done'
                ? t('pairing.doneHint')
                : t('pairing.codeHint')
          }}
        </p>
      </header>
      <div
        v-if="phase.kind === 'setup'"
        class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <SettingsRow :label="t('pairing.system')">
          <Segmented
            v-model="system"
            size="sm"
            :options="deviceSystems"
            :aria-label="t('pairing.systemLabel')"
          />
        </SettingsRow>
        <div class="flex flex-col gap-2 p-4">
          <p class="text-[12px] leading-4 text-fg-subtle">
            {{
              system === 'windows'
                ? t('pairing.runInPowerShell')
                : t('pairing.runInTerminal')
            }}
          </p>
          <CopyCode :code="command" :copy-label="t('pairing.copyInstall')" />
        </div>
      </div>
      <template v-else-if="phase.kind === 'code' || phase.kind === 'pairing'">
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <!-- A code is 32 characters, wider than a control beside the label may be. -->
          <SettingsRow :label="t('pairing.code')">
            <template #below>
              <TextInput
                v-model="code"
                focused
                :aria-label="t('pairing.code')"
                :placeholder="t('pairing.paste')"
                literal
                class="w-full font-mono"
                :disabled="phase.kind === 'pairing'"
              />
            </template>
          </SettingsRow>
        </div>
        <InlineError
          v-if="phase.kind === 'code' && phase.error"
          :message="phase.error.message"
          :request-id="phase.error.requestId"
        />
      </template>
      <div
        v-else
        class="flex items-center gap-2 rounded-xl border border-line bg-surface-float p-4 text-chrome text-fg"
      >
        <Check :size="14" class="shrink-0 text-on-success" />
        <TruncatedText :text="phase.device.name" />
      </div>
    </div>
    <template #footer>
      <Button
        v-if="phase.kind === 'done'"
        variant="primary"
        @click="emit('close')"
        >{{ uiText('common.done') }}</Button
      >
      <template v-else>
        <Button v-if="phase.kind === 'code'" @click="emit('back')"
          >{{ uiText('common.back') }}</Button
        >
        <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
        <Button
          v-if="phase.kind === 'setup'"
          variant="primary"
          @click="emit('next')"
          >{{ uiText('common.continue') }}</Button
        >
        <Button
          v-else
          variant="primary"
          :disabled="!code.trim()"
          :loading="phase.kind === 'pairing'"
          @click="submit"
          >{{ t('pairing.pair') }}</Button
        >
      </template>
    </template>
  </Dialog>
</template>
