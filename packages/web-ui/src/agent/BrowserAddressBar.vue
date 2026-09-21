<script setup lang="ts">
import { ArrowLeft, ArrowRight, RotateCw } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import TextInput from '../ui/TextInput.vue'

/**
 * The address bar both kinds of browser tab share
 * (`web-application.md` § Package responsibilities): a Host tab has its own
 * history, a local tab keeps a framed page's history to itself.
 */
withDefaults(
  defineProps<{
    address: string
    /** Why Back and Forward are unavailable, when they are. */
    historyReason?: string | null
    canReload?: boolean
  }>(),
  { historyReason: null, canReload: true },
)

const emit = defineEmits<{
  'update:address': [address: string]
  submit: []
  back: []
  forward: []
  reload: []
}>()
</script>

<template>
  <div class="flex h-11 shrink-0 items-center gap-1 px-2">
    <div class="flex shrink-0 items-center">
      <IconButton
        :icon="ArrowLeft"
        variant="ghost"
        aria-label="Back"
        :disabled="historyReason !== null"
        :disabled-reason="historyReason ?? undefined"
        @click="emit('back')"
      />
      <IconButton
        :icon="ArrowRight"
        variant="ghost"
        aria-label="Forward"
        :disabled="historyReason !== null"
        :disabled-reason="historyReason ?? undefined"
        @click="emit('forward')"
      />
      <IconButton
        :icon="RotateCw"
        variant="ghost"
        aria-label="Refresh"
        spin-on-click
        :disabled="!canReload"
        @click="emit('reload')"
      />
    </div>
    <TextInput
      class="ml-2 min-w-0 flex-1"
      :model-value="address"
      placeholder="Enter address"
      aria-label="Browser address"
      @update:model-value="emit('update:address', $event)"
      @keydown.enter="emit('submit')"
    />
    <!-- A trailing control stands as far from the address as the navigation group does. -->
    <div v-if="$slots.trailing" class="ml-2 flex shrink-0 items-center">
      <slot name="trailing" />
    </div>
  </div>
</template>
