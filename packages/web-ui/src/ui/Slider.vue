<script setup lang="ts">
import { computed, ref } from 'vue'
import { clamp } from '@demicodes/utils'

const props = withDefaults(defineProps<{
  modelValue: number
  min?: number
  max?: number
  step?: number
  disabled?: boolean
}>(), {
  min: 0,
  max: 100,
  step: 1,
})

const emit = defineEmits<{
  'update:modelValue': [value: number]
}>()

const trackRef = ref<HTMLElement>()

const span = computed(() => props.max - props.min)
const progress = computed(() => {
  if (span.value <= 0) return 0
  return clamp((props.modelValue - props.min) / span.value, 0, 1)
})

function snap(raw: number): number {
  const stepped = props.min + Math.round((raw - props.min) / props.step) * props.step
  const digits = props.step % 1 === 0 ? 0 : String(props.step).split('.')[1]?.length ?? 0
  const rounded = digits > 0 ? Number(stepped.toFixed(digits)) : stepped
  return clamp(rounded, props.min, props.max)
}

function valueFromClientX(clientX: number): number {
  const track = trackRef.value
  if (!track || span.value <= 0) return props.min
  const rect = track.getBoundingClientRect()
  if (rect.width <= 0) return props.min
  const ratio = clamp((clientX - rect.left) / rect.width, 0, 1)
  return snap(props.min + ratio * span.value)
}

function commit(clientX: number) {
  if (props.disabled) return
  const next = valueFromClientX(clientX)
  if (next !== props.modelValue) emit('update:modelValue', next)
}

function onPointerDown(event: PointerEvent) {
  if (props.disabled) return
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture(event.pointerId)
  commit(event.clientX)
}

function onPointerMove(event: PointerEvent) {
  const target = event.currentTarget as HTMLElement
  if (!target.hasPointerCapture(event.pointerId)) return
  commit(event.clientX)
}

function nudge(deltaSteps: number) {
  if (props.disabled) return
  emit('update:modelValue', snap(props.modelValue + deltaSteps * props.step))
}

function onKeydown(event: KeyboardEvent) {
  if (props.disabled) return
  if (event.key === 'ArrowRight' || event.key === 'ArrowUp') {
    event.preventDefault()
    nudge(1)
    return
  }
  if (event.key === 'ArrowLeft' || event.key === 'ArrowDown') {
    event.preventDefault()
    nudge(-1)
    return
  }
  if (event.key === 'Home') {
    event.preventDefault()
    emit('update:modelValue', props.min)
    return
  }
  if (event.key === 'End') {
    event.preventDefault()
    emit('update:modelValue', props.max)
  }
}
</script>

<template>
  <span
    class="slider inline-flex h-7 w-20 cursor-default items-center px-1.5 select-none"
    :class="disabled ? 'pointer-events-none opacity-40' : ''"
    role="slider"
    :aria-valuemin="min"
    :aria-valuemax="max"
    :aria-valuenow="modelValue"
    :aria-disabled="disabled || undefined"
    tabindex="0"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @keydown="onKeydown"
  >
    <span ref="trackRef" class="relative h-0.5 w-full">
      <span class="absolute inset-0 rounded-full bg-overlay/10" />
      <span
        class="absolute inset-y-0 left-0 rounded-full"
        :style="{ width: `${progress * 100}%`, background: 'var(--on-accent)' }"
      />
      <span
        class="slider-thumb absolute top-1/2 size-3 rounded-full bg-white shadow-sm ring-1 ring-line"
        :style="{ left: `${progress * 100}%` }"
      />
    </span>
  </span>
</template>

<style scoped>
.slider-thumb {
  transform: translate(-50%, -50%);
}
</style>
