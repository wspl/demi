<script setup lang="ts">
import { onMounted, ref } from 'vue'

const props = defineProps<{
  size?: number
  strokeWidth?: number
  trackClass?: string
  arcClass?: string
}>()

const svgSize = props.size ?? 14
const stroke = props.strokeWidth ?? 2.5
const radius = (svgSize - stroke) / 2
const circumference = 2 * Math.PI * radius
const cx = svgSize / 2
const cy = svgSize / 2

const arcRef = ref<SVGCircleElement>()

const dashMin = circumference * 0.1
const dashMax = circumference * 0.45

onMounted(() => {
  arcRef.value?.animate([
    { strokeDasharray: `${dashMin} ${circumference - dashMin}`, strokeDashoffset: '0' },
    { strokeDasharray: `${dashMax} ${circumference - dashMax}`, strokeDashoffset: `${circumference * -0.2}` },
    { strokeDasharray: `${dashMin} ${circumference - dashMin}`, strokeDashoffset: `${-circumference}` },
  ], {
    duration: 1500,
    easing: 'ease-in-out',
    iterations: Infinity,
  })
})
</script>

<template>
  <svg
    :width="svgSize" :height="svgSize"
    :viewBox="`0 0 ${svgSize} ${svgSize}`"
    class="spinner-rotate"
  >
    <circle
      :cx="cx" :cy="cy" :r="radius"
      fill="none"
      stroke="currentColor"
      :stroke-width="stroke"
      :class="trackClass ?? 'text-overlay/8'"
    />
    <circle
      ref="arcRef"
      :cx="cx" :cy="cy" :r="radius"
      fill="none"
      stroke="currentColor"
      :stroke-width="stroke"
      stroke-linecap="round"
      :class="arcClass ?? 'text-fg-muted'"
      :stroke-dasharray="`${dashMin} ${circumference - dashMin}`"
      :style="{ transformOrigin: `${cx}px ${cy}px` }"
    />
  </svg>
</template>

<style scoped>
.spinner-rotate {
  animation: spinner-rotate 2s linear infinite;
}

@keyframes spinner-rotate {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}
</style>
