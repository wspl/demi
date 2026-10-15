<script setup lang="ts">
import { t } from './locales/text'
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

const canAdd = computed(() => origin.value.trim().length > 0)

async function submit() {
  if (!canAdd.value)
    return
  const answer = await props.addSource({ origin: origin.value.trim() })
  if (answer.kind === 'added') {
    emit('close')
  } else if (answer.kind === 'refused') {
    refusal.value = answer.message
  }
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="wide"
    :label="t('addSource.title')"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('addSource.title') }}</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">{{ t('addSource.hint') }}</p>
      </header>

      <div
        class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <SettingsRow :label="t('addSource.repository')" :description="t('addSource.repositoryHint')">
          <TextInput
            v-model="origin"
            focused
            class="w-72 max-w-full"
            placeholder="owner/repository"
            :aria-label="t('addSource.repository')"
            literal
          />
        </SettingsRow>
      </div>
      <InlineError v-if="refusal" :message="refusal" />
    </div>
    <template #footer-leading>
      <ExternalLink href="https://skills.sh">{{ t('addSource.browse') }}</ExternalLink>
    </template>
    <template #footer>
      <Button @click="emit('close')">{{ t('addSource.cancel') }}</Button>
      <Button
        variant="primary"
        :disabled="!canAdd"
        @click="submit"
      >{{ t('addSource.title') }}</Button>
    </template>
  </Dialog>
</template>
