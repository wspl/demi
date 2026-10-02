<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '@demicodes/plugin-sdk'
import { Button } from '@demicodes/plugin-sdk'
import { Dialog } from '@demicodes/plugin-sdk'
import { ExternalLink } from '@demicodes/plugin-sdk'
import { TextInput } from '@demicodes/plugin-sdk'
import { SettingsRow } from '@demicodes/plugin-sdk'
import type { SettingsSkillDraft } from './types'

/**
 * Adding a skill source: a git repository. Every SKILL.md in it becomes a
 * skill the page can turn on.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  close: []
  add: [draft: SettingsSkillDraft]
}>()

const origin = ref('')

watch(() => props.isOpen, (open) => {
  if (!open)
    return
  origin.value = ''
})

const canAdd = computed(() => origin.value.trim().length > 0)

function submit() {
  if (!canAdd.value)
    return
  emit('add', { origin: origin.value.trim() })
  emit('close')
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="wide"
    label="Add source"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add source</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">A git repository. Every SKILL.md in it becomes a skill you can turn on.</p>
      </header>

      <div
        class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <SettingsRow label="Repository" description="owner/repo or a git URL.">
          <TextInput
            v-model="origin"
            focused
            class="w-72 max-w-full"
            placeholder="vercel-labs/agent-skills"
            @keydown.enter="submit"
          />
        </SettingsRow>
      </div>

      <div class="flex items-center justify-between gap-3">
        <ExternalLink href="https://skills.sh">Browse skills.sh</ExternalLink>
        <div class="flex justify-end gap-2">
          <Button @click="emit('close')">Cancel</Button>
          <Button
            variant="primary"
            :disabled="!canAdd"
            @click="submit"
          >Add source</Button>
        </div>
      </div>
    </div>
  </Dialog>
</template>
