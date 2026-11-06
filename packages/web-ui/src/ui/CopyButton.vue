<script setup lang="ts">
import { computed, ref } from 'vue'
import { Check, Copy } from '@lucide/vue'
import { useCopy, type CopySource } from '../composables/useCopy'
import IconButton from './IconButton.vue'
import { t as shared } from '../locales/text'
import Tooltip from './Tooltip.vue'
import type { SentenceText } from './ui-text'

/**
 * The one Copy control: an icon button with its tip that puts text, or a clipboard
 * item of richer content, on the clipboard. Every Copy in the app is this.
 *
 * - It turns to a check, a tip and an accessible name that say Copied for a moment.
 * - It tells the user when the clipboard refused (`useCopy`).
 */
const props = withDefaults(defineProps<{
  /**
   * What it copies, or a function that gives it as the control is pressed,
   * for content the page holds elsewhere; returning nothing copies nothing.
   */
  source?: string | (() => CopySource)
  /**
   * Text the page asks for as the control is pressed, such as a secret's
   * value, put on the clipboard as it arrives (`useCopy` § `copyWhenMade`)
   * in place of `source`; a rejection copies nothing, its maker having told
   * why.
   */
  made?: () => Promise<string>
  /** The name of the control and its tip, naming the object only where the control is away from it (Copy code). */
  label?: SentenceText
  /** A longer tip in place of the label, for a control whose object needs saying. */
  hint?: SentenceText
  size?: 'xs' | 'sm' | 'md' | 'lg'
  variant?: 'default' | 'ghost'
}>(), {
  size: 'sm',
  variant: 'ghost',
})

/** The text `made` last gave, which Copied stands for. */
const madeText = ref<string>()
const { copy, copyWhenMade, copied } = useCopy(() => {
  if (props.made)
    return madeText.value
  return typeof props.source === 'function' ? props.source() : props.source
})

function press(): void {
  const made = props.made
  if (!made) {
    void copy()
    return
  }
  void copyWhenMade(made().then((text) => {
    madeText.value = text
    return text
  }))
}
const tip = computed(() => props.label ?? shared('common.copy'))
const name = computed(() => (copied.value ? shared('common.copied') : tip.value))
</script>

<template>
  <Tooltip :content="copied ? shared('common.copied') : (hint ?? tip)">
    <IconButton
      :icon="copied ? Check : Copy"
      :size="size"
      :variant="variant"
      :aria-label="name"
      @click="press"
    />
  </Tooltip>
</template>
