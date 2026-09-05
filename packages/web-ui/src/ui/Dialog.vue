<script setup lang="ts">
import { computed, inject, provide, ref, watch } from 'vue'
import { onKeyStroke } from '@vueuse/core'
import { createOverlayFamily, overlayFamilyKey } from '../overlay/overlayFamily'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayContainerKey } from '../overlay/overlayContainer'
import { useOverlay } from '../composables/useOverlay'

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  size?: 'md' | 'lg'
  label?: string
}>()

const emit = defineEmits<{
  close: []
}>()

// A dialog confined to a host container never blocks the page, so it is not exclusive.
const container = inject(overlayContainerKey, null)
const teleportTarget = computed(() => container?.value ?? 'body')
const panel = ref<HTMLElement>()
const family = createOverlayFamily()
provide(overlayFamilyKey, family)
watch(panel, (element, _previous, onCleanup) => {
  if (element) onCleanup(family.register(element))
})
onKeyStroke('Escape', (event) => {
  if (!props.isOpen || container) return
  event.preventDefault()
  emit('close')
})

useOverlay(props.overlayStore, () => (container ? false : props.isOpen), () => {
  if (props.isOpen) emit('close')
})
</script>

<template>
  <Teleport :to="teleportTarget">
    <Transition
      enter-active-class="transition-opacity duration-150 ease-out"
      leave-active-class="transition-opacity duration-150 ease-out"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div
        v-if="isOpen"
        class="fixed inset-0 z-50 grid place-items-center bg-black/50"
        @click.self="emit('close')"
      >
        <Transition
          appear
          enter-active-class="transition-[opacity,scale] duration-150 ease-out"
          leave-active-class="transition-[opacity,scale] duration-150 ease-out"
          enter-from-class="opacity-0 scale-95"
          leave-to-class="opacity-0 scale-95"
        >
          <div
            ref="panel"
            class="overlay-shell max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] overflow-y-auto rounded-xl"
            :class="size === 'lg' ? 'max-w-3xl' : 'max-w-md'"
            role="dialog"
            aria-modal="true"
            :aria-label="label"
          >
            <slot />
          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
</template>
