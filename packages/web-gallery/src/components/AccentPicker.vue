<script setup lang="ts">
import SwatchPicker from '@demicodes/web-ui/ui/SwatchPicker.vue'
import { productAccentOptions } from '@demicodes/web-ui/theme/productAppearance'
import { t as ui } from '@demicodes/web-ui/locales/text'
import type { AccentId } from '../gallery-state'

defineProps<{
  modelValue: AccentId
}>()

const emit = defineEmits<{
  'update:modelValue': [value: AccentId]
}>()
</script>

<template>
  <div class="flex flex-col gap-1.5">
    <div class="gallery-label">{{ ui('settings.general.accent') }}</div>
    <SwatchPicker
      :model-value="modelValue"
      :options="productAccentOptions()"
      @update:model-value="emit('update:modelValue', $event as AccentId)"
    />
  </div>
</template>
