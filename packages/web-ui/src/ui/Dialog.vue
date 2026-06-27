<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import { useOverlay } from '../composables/useOverlay'

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  close: []
}>()

useOverlay(props.overlayStore, () => props.isOpen, () => {
  if (props.isOpen) emit('close')
})
</script>

<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition-opacity duration-150 ease-out"
      leave-active-class="transition-opacity duration-150 ease-out"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div
        v-if="isOpen"
        class="fixed inset-0 z-50 grid place-items-center bg-black/60"
        @click.self="emit('close')"
      >
        <Transition
          appear
          enter-active-class="transition-[opacity,transform] duration-150 ease-out"
          leave-active-class="transition-[opacity,transform] duration-150 ease-out"
          enter-from-class="opacity-0 scale-95"
          leave-to-class="opacity-0 scale-95"
        >
          <div
            class="w-full max-w-md rounded-xl border border-line bg-surface shadow-2xl"
          >
            <slot />
          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
</template>
