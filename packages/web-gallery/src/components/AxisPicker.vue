<script setup lang="ts" generic="T extends string">
import { computed } from 'vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'

/** One axis of the gallery's appearance or of a specimen: a label over the product's `Segmented`. */
const props = defineProps<{
  label: TitleText
  values: readonly T[]
  names?: Partial<Record<T, string>>
}>()

const model = defineModel<T>({ required: true })
const options = computed(() => props.values.map((value) => ({ value, label: props.names?.[value] ?? value })))
</script>

<template>
  <div class="flex flex-col gap-1.5">
    <div class="gallery-label">{{ label }}</div>
    <Segmented v-model="model" :options="options" size="sm" />
  </div>
</template>
