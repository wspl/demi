<script setup lang="ts">
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
    <div class="flex flex-wrap items-center gap-2">
      <button
        v-for="item in ACCENTS"
        :key="item.id"
        type="button"
        class="flex size-5 items-center justify-center rounded-full outline-offset-0 transition-[outline-color,background-color] duration-200 ease-out"
        :class="modelValue === item.id
          ? 'outline outline-[1.5px]'
          : 'hover:bg-hover'"
        :style="modelValue === item.id ? { outlineColor: item.swatch } : undefined"
        :title="item.name"
        :aria-label="item.name"
        :aria-pressed="modelValue === item.id"
        @click="emit('update:modelValue', item.id)"
      >
        <span
          class="size-3.5 rounded-full"
          :style="{ background: item.swatch }"
        />
      </button>
    </div>
  </div>
</template>
