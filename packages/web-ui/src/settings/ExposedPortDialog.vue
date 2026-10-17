<script setup lang="ts">
import { t } from '../devices/locales/text'
import { t as uiText } from '../locales/text'
import { ref, watch } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import CopyButton from '../ui/CopyButton.vue'
import Dialog from '../ui/Dialog.vue'
import InlineError from '../ui/InlineError.vue'
import { shownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import type { SentenceText } from '../ui/ui-text'
import SettingsRow from './SettingsRow.vue'
import { exposedPortProblem, type ExposedPortChange, type SettingsExposedPort } from './exposed-ports'

/**
 * An exposed port's fields, the one editor a new row and an existing one
 * open (create first, then edit): the port, shown as missing until it is
 * entered, its label, and the address it is reached at. Each field is saved
 * as it is committed; a refusal shows under its field, which keeps the
 * value to correct.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  exposed: SettingsExposedPort | null
  /** Saves a change; a refusal the user can fix is thrown, and its field shows it. */
  onChange: (change: ExposedPortChange) => Promise<void>
}>()
const emit = defineEmits<{
  close: []
}>()

const problems = ref<Partial<Record<keyof ExposedPortChange, SentenceText>>>({})
watch(
  () => props.exposed?.id,
  () => {
    problems.value = {}
  },
)

/** Saves a change and answers its refusal, which keeps the typed value in its field. */
async function save(change: ExposedPortChange): Promise<SentenceText | null> {
  try {
    await props.onChange(change)
    return null
  } catch (error) {
    return shownError(error).message
  }
}
</script>

<template>
  <Dialog :is-open="isOpen && !!exposed" :overlay-store="overlayStore" :label="t('exposed.editTitle')" @close="emit('close')">
    <div v-if="exposed" class="flex flex-col gap-4 px-5 pt-5">
      <h3 class="select-none pr-8 text-[15px] font-medium text-fg-emphasis">{{ t('exposed.editTitle') }}</h3>
      <div class="flex flex-col">
        <SettingsRow :label="t('exposed.port')" :description="t('exposed.portDescription')">
          <template v-if="problems.port" #detail>
            <InlineError :message="problems.port" />
          </template>
          <span class="flex items-center gap-1.5">
            <!-- Missing is no failure: the field says it, amber, until a port is entered. -->
            <TriangleAlert v-if="exposed.port === null && !problems.port" :size="ICON_PX.in24" class="shrink-0 text-on-warning" aria-hidden="true" />
            <CommitTextInput
              :model-value="exposed.port === null ? '' : String(exposed.port)"
              :aria-label="t('exposed.port')"
              :validate="exposedPortProblem"
              placeholder="11434"
              inputmode="numeric"
              trim
              class="w-28 max-w-full"
              :save="(value) => save({ port: value.trim() === '' ? null : Number(value.trim()) })"
              @problem="problems.port = $event ?? undefined"
            />
          </span>
        </SettingsRow>
        <SettingsRow :label="t('exposed.label')">
          <template v-if="problems.label" #detail>
            <InlineError :message="problems.label" />
          </template>
          <CommitTextInput
            :model-value="exposed.label"
            :aria-label="t('exposed.label')"
            :placeholder="t('exposed.labelPlaceholder')"
            trim
            class="w-44 max-w-full"
            :save="(value) => save({ label: value.trim() })"
            @problem="problems.label = $event ?? undefined"
          />
        </SettingsRow>
        <!-- The row stays while the port is missing, empty, so that entering
             one does not grow the dialog and move Done under the pointer. -->
        <SettingsRow :label="t('exposed.address')">
          <template v-if="exposed.address">
            <span class="min-w-0 truncate font-mono text-chrome text-fg-muted">{{ exposed.address }}</span>
            <CopyButton :source="exposed.address" :label="t('exposed.copy')" size="sm" />
          </template>
        </SettingsRow>
      </div>
    </div>
    <template #footer>
      <Button variant="primary" @click="emit('close')">{{ uiText('common.done') }}</Button>
    </template>
  </Dialog>
</template>
