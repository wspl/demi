<script setup lang="ts">
import { X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import { t } from '../locales/text'
import { useSheetDismiss } from './panel-sheet'

/**
 * The Close button a phone's work-panel overlay shows at the end of the view's
 * own bar, which closes the overlay to the conversation (`panel-sheet.ts`). It
 * renders nothing beside the conversation on a computer and in the full panel,
 * so a bar places it unconditionally and it appears only in an overlay.
 */
const dismiss = useSheetDismiss()
</script>

<template>
  <IconButton
    v-if="dismiss"
    :icon="X"
    variant="ghost"
    size="sm"
    class="shrink-0"
    :aria-label="t('common.close')"
    @click="dismiss?.()"
  />
</template>
