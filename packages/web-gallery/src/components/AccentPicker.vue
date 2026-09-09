<script setup lang="ts">
import SwatchPicker from '@demicodes/web-ui/ui/SwatchPicker.vue'
import { ACCENTS, type AccentId } from '../gallery-state'

defineProps<{
  modelValue: AccentId
}>()

const emit = defineEmits<{
  'update:modelValue': [value: AccentId]
}>()
</script>

<template>
  <div class="flex flex-col gap-1.5">
    <div class="gallery-label">Accent</div>
    <SwatchPicker
      :model-value="modelValue"
      :options="ACCENTS"
      @update:model-value="emit('update:modelValue', $event as AccentId)"
    />
  </div>
</template>
