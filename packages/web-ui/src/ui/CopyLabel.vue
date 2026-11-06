<script setup lang="ts">
import { Check, Copy } from '@lucide/vue'
import { t } from '../locales/text'

/**
 * A copy button's content: the copy mark and `label`, turned to a check and
 * Copied while `copied`. Both labels take one cell, so the button keeps the
 * wider one's width and nothing beside it moves.
 */
defineProps<{
  label: string
  copied: boolean
}>()
</script>

<template>
  <component :is="copied ? Check : Copy" :size="14" />
  <span class="inline-grid">
    <span class="col-start-1 row-start-1" :class="copied ? 'invisible' : ''">{{ label }}</span>
    <span class="col-start-1 row-start-1" :class="copied ? '' : 'invisible'">{{ t('common.copied') }}</span>
  </span>
</template>
