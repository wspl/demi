<script setup lang="ts">
import { t } from '../devices/locales/text'
import { t as uiText } from '../locales/text'
import { computed, ref } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import Button from '../ui/Button.vue'
import CopyButton from '../ui/CopyButton.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import SettingsGroup from './SettingsGroup.vue'
import SettingsRow from './SettingsRow.vue'
import ExposedPortDialog from './ExposedPortDialog.vue'
import { exposedPortName, type ExposedPortChange, type SettingsExposedPort } from './exposed-ports'

/**
 * A device's Exposed Ports (`port-exposure.md` § Exposed ports): each port
 * with its label, *11434 · Ollama*, and the address Demi's requests reach
 * it at, with Copy. Expose Port adds a row with the port missing and opens
 * its editor, the one Edit… opens; Remove stops exposing it.
 */
const props = defineProps<{
  /** The device's exposed ports, oldest first. */
  ports: readonly SettingsExposedPort[]
  overlayStore: OverlayStore
  /** Expose Port: makes a row with the port missing and answers its id, or null when it failed. */
  onExpose?: () => Promise<string | null>
  onChange?: (id: string, change: ExposedPortChange) => Promise<void>
  onRemove?: (id: string) => Promise<void>
}>()

/** The row whose editor is open. */
const editing = ref<string | null>(null)
const edited = computed(() => props.ports.find((exposed) => exposed.id === editing.value) ?? null)

/** A change in the open editor, to its row. */
async function changeEdited(change: ExposedPortChange): Promise<void> {
  if (edited.value) {
    await props.onChange?.(edited.value.id, change)
  }
}

async function expose(): Promise<void> {
  const id = await props.onExpose?.()
  if (id) {
    editing.value = id
  }
}
</script>

<template>
  <SettingsGroup :title="t('exposed.title')" :description="t('exposed.description')">
    <template #actions>
      <Button size="sm" @click="expose">{{ t('exposed.expose') }}</Button>
    </template>
    <SettingsRow v-if="!ports.length" :label="t('exposed.none')" muted />
    <SettingsRow
      v-for="exposed in ports"
      :key="exposed.id"
      :label="exposedPortName(exposed)"
      :target="`exposed-port:${exposed.id}`"
      compact
    >
      <template v-if="exposed.port === null" #leading>
        <TriangleAlert :size="ICON_PX.in24" class="text-on-warning" aria-hidden="true" />
      </template>
      <!-- The address is the row's second line, as a forward's route is, so the
           buttons center on the row and never outgrow it where they drop below. -->
      <template v-if="exposed.address" #description>
        <span class="flex min-w-0 items-center gap-1">
          <span class="min-w-0 truncate font-mono text-chrome text-fg-muted">{{ exposed.address }}</span>
          <CopyButton :source="exposed.address" :label="t('exposed.copy')" size="xs" />
        </span>
      </template>
      <Button size="sm" @click="editing = exposed.id">{{ t('exposed.edit') }}</Button>
      <Button size="sm" @click="onRemove?.(exposed.id)">{{ uiText('common.remove') }}</Button>
    </SettingsRow>
    <ExposedPortDialog
      :is-open="edited !== null"
      :overlay-store="overlayStore"
      :exposed="edited"
      :on-change="changeEdited"
      @close="editing = null"
    />
  </SettingsGroup>
</template>
