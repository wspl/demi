<script setup lang="ts">
import { ref } from 'vue'
import { ArrowDown, ArrowUp, X } from '@lucide/vue'
import TextInput from '../../../ui/TextInput.vue'
import Tooltip from '../../../ui/Tooltip.vue'
import Button from '../../../ui/Button.vue'

const props = defineProps<{
  search: string
  replace: string
  caseSensitive: boolean
  regexp: boolean
  matchCount: number
  currentMatch: number
  onSearchChange: (val: string) => void
  onReplaceChange: (val: string) => void
  onToggleCaseSensitive: () => void
  onToggleRegexp: () => void
  onNext: () => void
  onPrev: () => void
  onReplaceOne: () => void
  onReplaceAll: () => void
  onClose: () => void
  t: (key: string) => string
}>()

const searchRef = ref<InstanceType<typeof TextInput>>()

function onSearchKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter') {
    e.preventDefault()
    if (e.shiftKey) props.onPrev()
    else props.onNext()
  }
  if (e.key === 'Escape') {
    e.preventDefault()
    props.onClose()
  }
}

function onReplaceKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter') {
    e.preventDefault()
    props.onReplaceOne()
  }
  if (e.key === 'Escape') {
    e.preventDefault()
    props.onClose()
  }
}

defineExpose({ focusSearch() { searchRef.value?.focus(); searchRef.value?.select() } })
</script>

<template>
  <div class="relative flex items-start gap-2 border-t border-line-strong bg-surface-editor p-2" @keydown.stop>
    <!-- Close -->
    <span
      class="absolute right-1.5 top-[10px] flex size-[22px] cursor-pointer items-center justify-center rounded text-fg-subtle hover:bg-active hover:text-fg"
      :title="t('search.close')"
      @click="onClose()"
    >
      <X :size="14" />
    </span>

    <!-- Inputs column -->
    <div class="flex w-[60%] min-w-0 flex-col gap-1.5">
      <TextInput
        ref="searchRef"
        :model-value="props.search"
        :placeholder="props.t('search.label')"
        @update:model-value="onSearchChange($event)"
        @keydown="onSearchKeydown"
      >
        <template v-if="props.search" #suffix>
          <span class="text-[11px] tabular-nums text-fg-subtle">
            {{ matchCount > 0 ? `${currentMatch}/${matchCount}` : props.t('common.noResult') }}
          </span>
        </template>
      </TextInput>
      <TextInput
        :model-value="props.replace"
        :placeholder="props.t('search.replace')"
        @update:model-value="onReplaceChange($event)"
        @keydown="onReplaceKeydown"
      />
    </div>

    <!-- Controls column -->
    <div class="flex shrink-0 flex-col gap-1.5">
      <!-- Search controls row -->
      <div class="flex h-[26px] items-center gap-0.5">
        <Tooltip :content="props.t('search.matchCase')">
          <span
            :class="['flex size-[22px] cursor-pointer select-none items-center justify-center rounded text-[11px]', caseSensitive ? 'bg-overlay/8 text-fg-emphasis' : 'text-fg-subtle hover:text-fg-body']"
            @click="onToggleCaseSensitive()"
          >Aa</span>
        </Tooltip>
        <Tooltip :content="props.t('search.regex')">
          <span
            :class="['flex size-[22px] cursor-pointer select-none items-center justify-center rounded text-[11px]', regexp ? 'bg-overlay/8 text-fg-emphasis' : 'text-fg-subtle hover:text-fg-body']"
            @click="onToggleRegexp()"
          >.*</span>
        </Tooltip>

        <Tooltip :content="props.t('search.previousMatch')">
          <span
            class="flex size-[22px] cursor-pointer items-center justify-center rounded text-fg-muted hover:bg-active hover:text-fg"
            @click="onPrev()"
          >
            <ArrowUp :size="14" />
          </span>
        </Tooltip>
        <Tooltip :content="props.t('search.nextMatch')">
          <span
            class="flex size-[22px] cursor-pointer items-center justify-center rounded text-fg-muted hover:bg-active hover:text-fg"
            @click="onNext()"
          >
            <ArrowDown :size="14" />
          </span>
        </Tooltip>
      </div>

      <!-- Replace controls row -->
      <div class="flex h-[26px] items-center gap-1">
        <Button size="xs" @click="onReplaceOne()">{{ props.t('search.replace') }}</Button>
        <Button size="xs" @click="onReplaceAll()">{{ props.t('search.replaceAll') }}</Button>
      </div>
    </div>
  </div>
</template>
