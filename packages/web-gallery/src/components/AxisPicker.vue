<script setup lang="ts" generic="T extends string">
defineProps<{
  label: string
  values: readonly T[]
  modelValue: T
  names?: Partial<Record<T, string>>
}>()

const emit = defineEmits<{
  'update:modelValue': [value: T]
}>()
</script>

<template>
  <div class="flex flex-col gap-1.5">
    <div class="gallery-label">{{ label }}</div>
    <div class="flex flex-wrap gap-1">
      <button
        v-for="value in values"
        :key="value"
        type="button"
        class="rounded-md px-2 py-1 text-[12px] transition-colors duration-200 ease-out"
        :class="modelValue === value
          ? 'bg-active text-fg-emphasis'
          : 'text-fg-muted hover:bg-hover hover:text-fg'"
        @click="emit('update:modelValue', value)"
      >
        {{ names?.[value] ?? value }}
      </button>
    </div>
  </div>
</template>
