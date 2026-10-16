<script setup lang="ts">
import Tooltip from './Tooltip.vue'

/** A row of colour swatches as a radio group; the chosen one wears a ring in its own colour. */
defineProps<{
  modelValue: string
  options: readonly {
    id: string;
    name: string;
    swatch: string
  }[]
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()
</script>

<template>
  <!-- On a phone a swatch is a small button's 36 px, 8 px from the next, its colour 20 px. -->
  <div class="flex flex-wrap items-center gap-1.5 phone:gap-2" role="radiogroup">
    <Tooltip v-for="item in options" :key="item.id" :content="item.name" class="flex">
      <button
        type="button"
        role="radio"
        class="flex size-6 cursor-default items-center justify-center rounded-full outline-offset-0 transition-[outline-color,background-color] duration-200 ease-out phone:size-hit-sm"
        data-drawn-target
        :class="modelValue === item.id ? 'outline outline-[1.5px]' : 'hover:bg-hover'"
        :style="modelValue === item.id ? { outlineColor: item.swatch } : undefined"
        :aria-label="item.name"
        :aria-checked="modelValue === item.id"
        @click="emit('update:modelValue', item.id)"
      >
        <span
          class="size-3.5 rounded-full phone:size-5"
          :style="{ background: item.swatch }"
        />
      </button>
    </Tooltip>
  </div>
</template>
