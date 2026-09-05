<script setup lang="ts">
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/** Activity marks under exploration. The product draws only ActivityMark's sweep. */
export type ExploratoryMarkKind = 'pulse' | 'dots' | 'signal' | 'orbit' | 'cluster'

withDefaults(defineProps<{
  kind: ExploratoryMarkKind
  size?: number
}>(), {
  size: ICON_PX.in28,
})
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    class="exploratory-mark"
    aria-hidden="true"
  >
    <template v-if="kind === 'pulse'">
      <circle class="mark-pulse-ring" cx="12" cy="12" r="8.25" />
      <circle class="mark-pulse-core" cx="12" cy="12" r="3.25" />
    </template>
    <template v-else-if="kind === 'dots'">
      <circle class="mark-dot mark-dot-a" cx="6" cy="12" r="1.7" />
      <circle class="mark-dot mark-dot-b" cx="12" cy="12" r="1.7" />
      <circle class="mark-dot mark-dot-c" cx="18" cy="12" r="1.7" />
    </template>
    <template v-else-if="kind === 'signal'">
      <circle class="mark-signal mark-signal-a" cx="12" cy="12" r="3" />
      <circle class="mark-signal mark-signal-b" cx="12" cy="12" r="3" />
      <circle class="mark-signal-core" cx="12" cy="12" r="1.4" />
    </template>
    <template v-else-if="kind === 'orbit'">
      <circle class="mark-track" cx="12" cy="12" r="8.25" />
      <circle class="mark-orbit-dot mark-spin" cx="12" cy="3.75" r="1.7" />
    </template>
    <template v-else>
      <!-- A tilted ring seen from the side: the group turns at one speed, each dot only breathes
           in size and opacity as its angle carries it to the front (bottom) and the back (top). -->
      <g class="mark-cluster-ring">
        <circle class="mark-cluster-dot mark-cluster-a" cx="12" cy="18" r="2.6" />
        <circle class="mark-cluster-dot mark-cluster-b" cx="17.2" cy="9" r="2.6" />
        <circle class="mark-cluster-dot mark-cluster-c" cx="6.8" cy="9" r="2.6" />
      </g>
    </template>
  </svg>
</template>

<style scoped>
.exploratory-mark {
  display: block;
  overflow: visible;
}

.mark-track,
.mark-pulse-ring,
.mark-signal {
  fill: none;
  stroke: currentColor;
  stroke-width: var(--icon-stroke-width);
}

.mark-track,
.mark-pulse-ring {
  opacity: 0.18;
}

.mark-spin {
  transform-box: view-box;
  transform-origin: 12px 12px;
  animation: mark-spin 0.9s linear infinite;
}

.mark-cluster-ring {
  transform-box: view-box;
  transform-origin: 12px 12px;
  animation: mark-spin 3s linear infinite;
}

.mark-cluster-dot {
  fill: currentColor;
  transform-box: fill-box;
  transform-origin: center;
  animation: mark-cluster-depth 3s ease-in-out infinite;
}

/* Each dot is at the front one third of a turn after the previous one. */
.mark-cluster-b { animation-delay: -2s; }
.mark-cluster-c { animation-delay: -1s; }

.mark-pulse-core,
.mark-dot,
.mark-orbit-dot,
.mark-signal-core {
  fill: currentColor;
}

.mark-pulse-core {
  transform-origin: 12px 12px;
  animation: mark-pulse 1.2s ease-in-out infinite;
}

.mark-dot {
  animation: mark-dot 1.2s ease-in-out infinite;
}

.mark-dot-b { animation-delay: 0.15s; }
.mark-dot-c { animation-delay: 0.3s; }

.mark-signal {
  stroke-linecap: round;
  transform-box: view-box;
  transform-origin: 12px 12px;
  animation: mark-signal 1.6s ease-out infinite;
}

.mark-signal-b { animation-delay: 0.55s; }

.mark-signal-core {
  animation: mark-pulse 1.6s ease-in-out infinite;
  transform-origin: 12px 12px;
}

@keyframes mark-spin {
  to { transform: rotate(360deg); }
}

/* Front at 0%, back at 50%: one smooth breath per turn, in step with the ring's angle. */
@keyframes mark-cluster-depth {
  0%, 100% {
    transform: scale(1);
    opacity: 1;
  }
  50% {
    transform: scale(0.5);
    opacity: 0.3;
  }
}

@media (prefers-reduced-motion: reduce) {
  .mark-cluster-ring,
  .mark-cluster-dot {
    animation: none;
  }

  .mark-cluster-b,
  .mark-cluster-c {
    transform: scale(0.65);
    opacity: 0.5;
  }
}

@keyframes mark-pulse {
  0%, 100% { transform: scale(0.72); opacity: 0.4; }
  50% { transform: scale(1); opacity: 1; }
}

@keyframes mark-dot {
  0%, 80%, 100% { opacity: 0.28; }
  40% { opacity: 1; }
}

@keyframes mark-signal {
  0% { transform: scale(0.45); opacity: 0.55; }
  100% { transform: scale(1.15); opacity: 0; }
}
</style>
