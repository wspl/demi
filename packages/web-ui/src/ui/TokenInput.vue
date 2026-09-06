<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import TextInput from './TextInput.vue'

/**
 * A token count typed in thousands or millions. The unit sits inside the field and
 * toggles on click; the model value is always whole tokens.
 */
const props = defineProps<{
  modelValue: number | null
  placeholder?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: number | null]
}>()

type Unit = 'K' | 'M'
const SCALE: Record<Unit, number> = { K: 1_000, M: 1_000_000 }

const unit = ref<Unit>(props.modelValue !== null && props.modelValue >= 1_000_000 ? 'M' : 'K')
const text = ref(display(props.modelValue, unit.value))

function display(tokens: number | null, u: Unit): string {
  if (tokens === null) return ''
  const n = tokens / SCALE[u]
  return Number.isInteger(n) ? String(n) : String(Number(n.toFixed(3)))
}

// A value set from outside re-renders in the current unit; local typing does not echo.
watch(() => props.modelValue, (tokens) => {
  if (numberOrNull(text.value, unit.value) !== tokens) text.value = display(tokens, unit.value)
})

function numberOrNull(value: string, u: Unit): number | null {
  const trimmed = value.replace(/[,\s]/g, '')
  if (!trimmed) return null
  const n = Number(trimmed)
  return Number.isFinite(n) && n >= 0 ? Math.round(n * SCALE[u]) : null
}

function onInput(value: string) {
  text.value = value
  emit('update:modelValue', numberOrNull(value, unit.value))
}

function toggleUnit() {
  const next: Unit = unit.value === 'K' ? 'M' : 'K'
  const tokens = numberOrNull(text.value, unit.value)
  unit.value = next
  text.value = display(tokens, next)
}

const otherUnit = computed(() => (unit.value === 'K' ? 'M' : 'K'))
</script>

<template>
  <TextInput :model-value="text" :placeholder="placeholder" inputmode="decimal" @update:model-value="onInput">
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
