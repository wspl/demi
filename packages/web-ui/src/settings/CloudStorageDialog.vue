<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import InlineError from '../ui/InlineError.vue'
import { formatCount } from '../ui/numbers'
import type { Action, Operation } from '../ui/operations'
import TextInput from '../ui/TextInput.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsCloudStorage } from './types'
import { CLOUD_GB_MAX, parseCloudGb } from './users'

/**
 * Cloud Storage…: the size of an account's Cloud disks, System and Home, in whole GB
 * from 1 to 1024.
 *
 * The sizes take effect the next time the Cloud starts, and a disk already larger
 * keeps its size, which the dialog says. Save waits for two sizes and a change.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The account's nickname, or its email until it has one. */
  name: string
  /** The sizes it has now. */
  current: SettingsCloudStorage
  /** Sets the sizes; Save works until it settles. */
  onSubmit?: Action<[storage: SettingsCloudStorage]>
  /** What Save does, to which account (`operations.ts`). */
  operation?: Operation
}>()

const emit = defineEmits<{
  close: []
}>()

const system = ref('')
const home = ref('')
// Each opening starts from the account's sizes.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      system.value = String(props.current.systemGb)
      home.value = String(props.current.homeGb)
    }
  },
  { immediate: true },
)

const systemGb = computed(() => parseCloudGb(system.value))
const homeGb = computed(() => parseCloudGb(home.value))
const canSubmit = computed(
  () =>
    systemGb.value !== null &&
    homeGb.value !== null &&
    (systemGb.value !== props.current.systemGb || homeGb.value !== props.current.homeGb),
)

function submit(): unknown {
  if (canSubmit.value && systemGb.value !== null && homeGb.value !== null) {
    return props.onSubmit?.({ systemGb: systemGb.value, homeGb: homeGb.value })
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="t('cloudStorage.title')" @close="emit('close')">
    <form class="flex flex-col gap-4 px-5 pt-5" @submit.prevent="submit">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('cloudStorage.title') }}</h3>
        <p class="mt-0.5 text-explanation leading-5 text-fg-muted">
          {{ t('cloudStorage.account', { name }) }}
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow :label="t('cloudStorage.system')" :description="t('cloudStorage.systemDescription')">
          <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
            <TextInput
              v-model="system"
              :aria-label="t('cloudStorage.system')"
              inputmode="numeric"
              literal
              trim
              focused
              :invalid="systemGb === null"
              class="w-20"
            />
            GB
          </span>
        </SettingsRow>
        <SettingsRow :label="t('cloudStorage.home')" :description="t('cloudStorage.homeDescription')">
          <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
            <TextInput
              v-model="home"
              :aria-label="t('cloudStorage.home')"
              inputmode="numeric"
              literal
              trim
              :invalid="homeGb === null"
              class="w-20"
            />
            GB
          </span>
        </SettingsRow>
      </div>
      <InlineError
        v-if="systemGb === null || homeGb === null"
        :message="t('cloudStorage.problem', { max: formatCount(CLOUD_GB_MAX) })"
      />
      <p v-else class="select-none text-explanation leading-5 text-fg-muted">
        {{ t('cloudStorage.note') }}
      </p>
    </form>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button variant="primary" :disabled="!canSubmit" :operation="operation" @click="submit">{{ uiText('common.save') }}</Button>
    </template>
  </Dialog>
</template>
