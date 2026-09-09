<script setup lang="ts">
import { ref } from 'vue'
import { ChevronsDownUp } from '@lucide/vue'
import { onDismissOutside } from '../overlay/dismissOutside'
import IconButton from '../ui/IconButton.vue'
import TabStrip from './TabStrip.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    /** False for a pinned catalog specimen: the window does not steal page clicks. */
    dismissOutside?: boolean
  }>(),
  {
    dismissOutside: true,
  },
)

const emit = defineEmits<{
  close: []
}>()

const panelRef = ref<HTMLElement | null>(null)

onDismissOutside(
  () => (props.open && props.dismissOutside ? panelRef.value : null),
  () => emit('close'),
  { ignore: ['[data-overlay-panel]', '[data-session-overlay-toggle]'] },
)
</script>

<template>
  <Transition
    enter-active-class="origin-bottom transition-[opacity,scale] duration-150 ease-out motion-reduce:transition-none"
    leave-active-class="origin-bottom transition-[opacity,scale] duration-150 ease-out motion-reduce:transition-none"
    enter-from-class="opacity-0 scale-95"
    leave-to-class="opacity-0 scale-95"
    appear
  >
    <div
      v-if="open"
      ref="panelRef"
      class="overlay-window pointer-events-auto absolute inset-0 z-10 flex min-h-0 origin-bottom flex-col overflow-hidden rounded-xl bg-surface"
    >
      <div class="flex h-10 shrink-0 items-center gap-1 bg-surface-base px-1.5">
        <TabStrip class="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto">
          <slot name="tabs" />
        </TabStrip>
        <slot name="trailing" />
        <IconButton
          :icon="ChevronsDownUp"
          size="sm"
          variant="ghost"
          aria-label="Fold"
          @click="emit('close')"
        />
      </div>
      <div class="min-h-0 flex-1 overflow-hidden bg-surface">
        <slot />
      </div>
    </div>
  </Transition>
</template>
