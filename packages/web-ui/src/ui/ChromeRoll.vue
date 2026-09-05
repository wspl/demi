<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, useSlots, watch } from 'vue'
import { CHROME_ROLL_MS } from './chrome-roll'

const props = defineProps<{
  faceKey: string
  /** When this stays put, only the label rolls. When it changes, icon and label roll together. */
  iconKey?: string
}>()

type RollMode = 'label' | 'face'

const slots = useSlots()
const iconPresent = computed(() => !!slots.icon)
/** The standing icon cell while only the label rolls. */
const iconRef = ref<HTMLElement>()
/** The element whose children are the current face: the label, or icon plus label. */
const faceRef = ref<HTMLElement>()
const outgoingRef = ref<HTMLElement>()
const outgoing = shallowRef<HTMLElement | null>(null)
const rolling = ref(false)
const rollMode = ref<RollMode>('label')
let rollTimer: ReturnType<typeof setTimeout> | undefined

function cloneChildren(el: HTMLElement | undefined): Node[] {
  return el ? [...el.childNodes].map((node) => node.cloneNode(true)) : []
}

// Snapshots come from the DOM still showing the old face (this runs before the re-render),
// so ordinary updates cost nothing: slots are never re-invoked to keep a copy warm.
function snapshotFace(mode: RollMode): HTMLElement | null {
  const face = faceRef.value
  if (!face) return null
  const wrap = document.createElement('div')
  wrap.className = 'flex h-7 items-center gap-2'
  if (mode === 'face' && rollMode.value === 'label') wrap.append(...cloneChildren(iconRef.value))
  wrap.append(...cloneChildren(face))
  return wrap.childNodes.length > 0 ? wrap : null
}

function clearRollTimer(): void {
  if (!rollTimer) return
  clearTimeout(rollTimer)
  rollTimer = undefined
}

function finishRoll(): void {
  outgoing.value = null
  rolling.value = false
}

function startRoll(mode: RollMode, snapshot: HTMLElement): void {
  clearRollTimer()
  if (rolling.value) finishRoll()
  rollMode.value = mode
  outgoing.value = snapshot
  rolling.value = false
  void nextTick(() => {
    outgoingRef.value?.replaceChildren(snapshot)
    rolling.value = true
    rollTimer = setTimeout(() => {
      rollTimer = undefined
      finishRoll()
    }, CHROME_ROLL_MS)
  })
}

watch(
  () => [props.faceKey, props.iconKey ?? ''] as const,
  ([, icon], [, prevIcon]) => {
    const mode: RollMode = icon === prevIcon && iconPresent.value ? 'label' : 'face'
    const snapshot = snapshotFace(mode)
    if (snapshot) startRoll(mode, snapshot)
  },
  { flush: 'pre' },
)

onBeforeUnmount(clearRollTimer)
</script>

<template>
  <div class="chrome-roll flex min-w-0 items-center gap-2" :style="{ '--chrome-roll-ms': `${CHROME_ROLL_MS}ms` }">
    <template v-if="iconPresent && rollMode === 'label'">
      <div ref="iconRef" class="flex h-7 shrink-0 items-center">
        <slot name="icon" />
      </div>
      <div class="chrome-roll-clip min-w-0 flex-1">
        <div class="chrome-roll-track" :class="rolling ? 'is-rolling' : ''">
          <div v-if="outgoing" ref="outgoingRef" class="chrome-roll-face" />
          <div ref="faceRef" class="chrome-roll-face">
            <slot />
          </div>
        </div>
      </div>
    </template>
    <div v-else class="chrome-roll-clip min-w-0 flex-1">
      <div class="chrome-roll-track" :class="rolling ? 'is-rolling' : ''">
        <div v-if="outgoing" ref="outgoingRef" class="chrome-roll-face" />
        <div class="chrome-roll-face">
          <div ref="faceRef" class="flex h-7 items-center gap-2">
            <slot v-if="iconPresent" name="icon" />
            <slot />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chrome-roll {
  height: calc(var(--spacing) * 7);
  max-height: calc(var(--spacing) * 7);
  overflow: hidden;
  contain: layout;
}

.chrome-roll-clip {
  height: 100%;
  overflow: hidden;
}

.chrome-roll-face {
  height: calc(var(--spacing) * 7);
  max-height: calc(var(--spacing) * 7);
  min-height: 0;
  overflow: hidden;
}

.chrome-roll-track {
  transform: translateY(0);
}

.chrome-roll-track.is-rolling {
  transition: transform var(--chrome-roll-ms) cubic-bezier(0.4, 0, 0.2, 1);
  transform: translateY(-50%);
}
</style>
