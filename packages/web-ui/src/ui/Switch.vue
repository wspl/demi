<script setup lang="ts">
withDefaults(defineProps<{
  modelValue: boolean
  label?: string
  size?: 'sm' | 'md'
}>(), {
  size: 'md',
})

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
}>()
</script>

<template>
  <span
    class="inline-flex cursor-pointer items-center gap-1.5"
    role="switch"
    :aria-checked="modelValue"
    @click="emit('update:modelValue', !modelValue)"
  >
    <span v-if="label" class="select-none text-[12px] text-fg-subtle">{{ label }}</span>
    <span
      class="relative inline-flex shrink-0 rounded-full transition-colors duration-150"
      :class="[
        modelValue ? 'bg-emerald-500' : 'bg-overlay/10',
        size === 'sm' ? 'h-3.5 w-6' : 'h-5 w-9',
      ]"
    >
      <span
        class="absolute rounded-full bg-white shadow-sm transition-all duration-150"
        :class="[
          size === 'sm' ? 'top-[2px] size-2.5' : 'top-0.5 size-4',
          size === 'sm'
            ? (modelValue ? 'left-[12px]' : 'left-[2px]')
            : (modelValue ? 'left-[18px]' : 'left-0.5'),
        ]"
      />
    </span>
  </span>
</template>
