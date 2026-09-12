<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import TextInput from './TextInput.vue'
import { displayTokenCount, parseTokenCount, type TokenUnit } from './token-count'

/**
 * A token count typed in thousands or millions. The unit sits inside the field and
 * toggles on click; the model value is always whole tokens.
 */
const props = defineProps<{
  modelValue: number | null
  placeholder?: string
  size?: 'sm' | 'md'
}>()

const emit = defineEmits<{
  'update:modelValue': [value: number | null]
}>()

const unit = ref<TokenUnit>(
  props.modelValue !== null && props.modelValue >= 1_000_000 ? 'M' : 'K'
)
const text = ref(displayTokenCount(props.modelValue, unit.value))

// A value set from outside re-renders in the current unit; local typing does not echo.
watch(() => props.modelValue, (tokens) => {
  if (parseTokenCount(text.value, unit.value) !== tokens)
    text.value = displayTokenCount(tokens, unit.value)
})

function onInput(value: string) {
  text.value = value
  emit('update:modelValue', parseTokenCount(value, unit.value))
}

function toggleUnit() {
  const next: TokenUnit = unit.value === 'K' ? 'M' : 'K'
  const tokens = parseTokenCount(text.value, unit.value)
  unit.value = next
  text.value = displayTokenCount(tokens, next)
}

const otherUnit = computed(() => (unit.value === 'K' ? 'M' : 'K'))
</script>

<template>
  <TextInput
    :model-value="text"
    :placeholder="placeholder"
    :size="size"
    inputmode="decimal"
    @update:model-value="onInput"
  >
    <template #suffix>
      <button
        type="button"
        class="h-5 min-w-5 cursor-default select-none rounded px-1 text-[11px] font-medium tabular-nums text-fg-muted transition-colors duration-200 ease-out hover:bg-hover hover:text-fg"
        :aria-label="`Unit: ${unit === 'K' ? 'thousand' : 'million'} tokens. Switch to ${otherUnit === 'K' ? 'thousand' : 'million'}`"
        @click="toggleUnit"
      >
        {{ unit }}
      </button>
    </template>
  </TextInput>
</template>
