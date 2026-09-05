<script setup lang="ts">
import { dismissToast, toasts } from '../infra/toast'
import Toast from './Toast.vue'

const overlayMotion = {
  enterActiveClass: 'transition-[opacity,transform] duration-150 ease-out',
  leaveActiveClass: 'transition-[opacity,transform] duration-150 ease-out',
  enterFromClass: 'translate-y-1 opacity-0',
  leaveToClass: 'translate-y-1 opacity-0',
  moveClass: 'transition-transform duration-150 ease-out',
}
</script>

<template>
  <Teleport to="body">
    <div
      class="pointer-events-none fixed right-4 bottom-4 z-50 flex w-80 max-w-[calc(100vw-2rem)] flex-col-reverse gap-2"
    >
      <TransitionGroup v-bind="overlayMotion">
        <div
          v-for="toast in toasts"
          :key="toast.id"
          class="pointer-events-auto"
        >
          <Toast
            :title="toast.title"
            :message="toast.message"
            :tone="toast.tone"
            @dismiss="dismissToast(toast.id)"
          />
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>
