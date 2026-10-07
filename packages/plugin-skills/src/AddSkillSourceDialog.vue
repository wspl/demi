<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '@demicodes/plugin-sdk'
import { Button } from '@demicodes/plugin-sdk'
import { Dialog } from '@demicodes/plugin-sdk'
import { ExternalLink } from '@demicodes/plugin-sdk'
import { InlineError } from '@demicodes/plugin-sdk'
import { TextInput } from '@demicodes/plugin-sdk'
import { SettingsRow } from '@demicodes/plugin-sdk'
import type { AddSourceAnswer, SettingsSkillDraft } from './types'

/**
 * Adding a skill source: a git repository. Every SKILL.md in it becomes a
 * skill the page can turn on. The dialog stays open until the source is
 * added; an origin the plugin refuses is said under the field, which keeps
 * what was typed.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  addSource: (draft: SettingsSkillDraft) => Promise<AddSourceAnswer>
}>()

const emit = defineEmits<{
  close: []
}>()

const origin = ref('')
const busy = ref(false)
const refusal = ref<string | null>(null)

watch(() => props.isOpen, (open) => {
  if (!open)
    return
  origin.value = ''
  refusal.value = null
})

// A changed origin is a new attempt: the refusal of the last one no longer applies.
watch(origin, () => {
  refusal.value = null
})

const canAdd = computed(() => !busy.value && origin.value.trim().length > 0)

async function submit() {
  if (!canAdd.value)
    return
  busy.value = true
  try {
    const answer = await props.addSource({ origin: origin.value.trim() })
    if (answer.kind === 'added') {
      emit('close')
    } else if (answer.kind === 'refused') {
      refusal.value = answer.message
    }
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="wide"
    label="Add Source"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add Source</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">A Git repository. Every SKILL.md in it becomes a skill you can turn on.</p>
      </header>

      <div
        class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <SettingsRow label="Repository" description="owner/repo or a Git URL.">
          <TextInput
            v-model="origin"
            focused
            class="w-72 max-w-full"
            placeholder="vercel-labs/agent-skills"
            aria-label="Repository"
            :readonly="busy"
            @keydown.enter="submit"
          />
        </SettingsRow>
      </div>
      <InlineError v-if="refusal" :message="refusal" />

      <div class="flex items-center justify-between gap-3">
        <ExternalLink href="https://skills.sh">Browse skills.sh</ExternalLink>
        <div class="flex justify-end gap-2">
          <Button @click="emit('close')">Cancel</Button>
          <Button
            variant="primary"
            :disabled="!canAdd && !busy"
            :loading="busy"
            @click="submit"
          >Add Source</Button>
        </div>
      </div>
    </div>
  </Dialog>
</template>
