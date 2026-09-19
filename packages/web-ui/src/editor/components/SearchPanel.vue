<script setup lang="ts">
import { computed, type Component } from 'vue'
import { ArrowDown, ArrowUp, CaseSensitive, Regex, WholeWord, X } from '@lucide/vue'
import IconButton from '../../ui/IconButton.vue'
import TextInput from '../../ui/TextInput.vue'
import Tooltip from '../../ui/Tooltip.vue'
import type { FindOptions } from '../searchPanel'

/**
 * The code view's find bar, below the text: the query with its match count,
 * how it matches, and steps between matches. The editor owns the search; the
 * bar shows it and reports what the user changes.
 */
const props = defineProps<{
  options: FindOptions
  /** The match the selection is on, from 1; 0 when it is on none. */
  current: number
  count: number
  /** False when counting stopped at its limit. */
  complete: boolean
}>()

const emit = defineEmits<{
  change: [options: FindOptions]
  previous: []
  next: []
  close: []
}>()

type Toggle = 'caseSensitive' | 'wholeWord' | 'regexp'

const toggles: readonly { key: Toggle; icon: Component; label: string }[] = [
  { key: 'caseSensitive', icon: CaseSensitive, label: 'Match case' },
  { key: 'wholeWord', icon: WholeWord, label: 'Match whole word' },
  { key: 'regexp', icon: Regex, label: 'Use regular expression' },
]

const status = computed(() => props.count === 0
  ? 'No results'
  : `${props.current || '?'}/${props.count}${props.complete ? '' : '+'}`)

function flip(key: Toggle): void {
  const next = { ...props.options }
  next[key] = !next[key]
  emit('change', next)
}
</script>

<template>
  <div class="flex h-9 items-center gap-1 border-t border-line bg-surface-editor px-1.5">
    <TextInput
      class="min-w-0 max-w-72 flex-1"
      size="sm"
      placeholder="Find"
      aria-label="Find"
      main-field="true"
      :model-value="options.search"
      @update:model-value="emit('change', { ...options, search: $event })"
    >
      <template v-if="options.search" #suffix>
        <span class="whitespace-nowrap text-[11px] tabular-nums text-fg-subtle" aria-live="polite">{{ status }}</span>
      </template>
    </TextInput>
    <Tooltip v-for="toggle in toggles" :key="toggle.key" :content="toggle.label">
      <IconButton
        :icon="toggle.icon"
        size="sm"
        variant="ghost"
        :pressed="options[toggle.key]"
        :aria-label="toggle.label"
        @click="flip(toggle.key)"
      />
    </Tooltip>
    <Tooltip content="Previous match">
      <IconButton :icon="ArrowUp" size="sm" variant="ghost" aria-label="Previous match" :disabled="count === 0" @click="emit('previous')" />
    </Tooltip>
    <Tooltip content="Next match">
      <IconButton :icon="ArrowDown" size="sm" variant="ghost" aria-label="Next match" :disabled="count === 0" @click="emit('next')" />
    </Tooltip>
    <Tooltip content="Close" class="ml-auto">
      <IconButton :icon="X" size="sm" variant="ghost" aria-label="Close" @click="emit('close')" />
    </Tooltip>
  </div>
</template>
