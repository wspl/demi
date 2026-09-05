<script setup lang="ts">
import { computed, inject } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayContainerKey } from '../overlay/overlayContainer'
import { useOverlay } from '../composables/useOverlay'

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  close: []
}>()

// A dialog confined to a host container never blocks the page, so it is not exclusive.
const container = inject(overlayContainerKey, null)
const teleportTarget = computed(() => container?.value ?? 'body')

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
            class="overlay-shell w-full max-w-md rounded-xl"
          >
            <slot />
          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
</template>
