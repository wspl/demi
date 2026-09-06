<script setup lang="ts">
/** A row of colour swatches as a radio group; the chosen one wears a ring in its own colour. */
defineProps<{
  modelValue: string
  options: readonly { id: string; name: string; swatch: string }[]
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()
</script>

<template>
  <div class="flex flex-wrap items-center gap-1.5" role="radiogroup">
    <button
      v-for="item in options"
      :key="item.id"
      type="button"
      role="radio"
      class="flex size-6 cursor-default items-center justify-center rounded-full outline-offset-0 transition-[outline-color,background-color] duration-200 ease-out"
      :class="modelValue === item.id ? 'outline outline-[1.5px]' : 'hover:bg-hover'"
      :style="modelValue === item.id ? { outlineColor: item.swatch } : undefined"
      :title="item.name"
      :aria-label="item.name"
      :aria-checked="modelValue === item.id"
      @click="emit('update:modelValue', item.id)"
    >
      <span class="size-3.5 rounded-full" :style="{ background: item.swatch }" />
    </button>
  </div>
</template>
