<script setup lang="ts">
import { computed, watch } from 'vue'
import { RegionStatus } from '@demicodes/plugin-sdk'
import type { MobileDevicesController } from './controller'
import { t } from '../locales/text'

/**
 * What the simulators' window says over its tabs while the user's choice in
 * the taskbar's Open opens: the latest one chosen, until its tab is there.
 * A refused open says `failed` once no other is still opening, so a window
 * it left empty closes and one that waits for another device stays.
 */
const props = defineProps<{
  controller: MobileDevicesController
  /** The devices whose tabs the panel holds now. */
  shown: ReadonlySet<string>
}>()
const emit = defineEmits<{ failed: [] }>()

// The tab may come before or after the open answers, so both are watched.
watch([() => props.shown, props.controller.openings], ([devices]) => props.controller.shown(devices), { immediate: true })
watch(props.controller.refusals, () => {
  if (props.controller.openings.value.length === 0) {
    emit('failed')
  }
})

const latest = computed(() => props.controller.openings.value.at(-1) ?? null)
</script>

<template>
  <RegionStatus
    v-if="latest"
    class="absolute inset-0 z-10 bg-surface"
    status="loading"
    :label="latest.boots ? t('open.booting', { name: latest.name }) : t('open.opening', { name: latest.name })"
  />
</template>
