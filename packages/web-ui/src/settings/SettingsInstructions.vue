<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import Button from '../ui/Button.vue'
import TextArea from '../ui/TextArea.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'

/**
 * The Instructions section (`instructions.md` § Personal instructions): one
 * text every conversation's agents follow, beside each project's AGENTS.md
 * or CLAUDE.md. The draft follows the saved text until the user edits it;
 * Save is offered while the draft differs and fits, and an empty draft
 * removes the instructions. The host saves.
 */
const props = defineProps<{
  /** The saved text; empty when the user has none. */
  text: string
  save: (text: string) => Promise<void>
}>()

/** The most characters the backend takes. */
const LIMIT = 65_536

const draft = ref(props.text)
const status = ref<{ kind: 'idle' | 'saving' } | { kind: 'failed'; message: string }>({ kind: 'idle' })

// Another page's save, or this one's answer, replaces a draft the user has not changed.
let shown = props.text
watch(() => props.text, (saved) => {
  if (draft.value === shown) {
    draft.value = saved
  }
  shown = saved
})

const length = computed(() => [...draft.value].length)
const tooLong = computed(() => length.value > LIMIT)
const changed = computed(() => draft.value !== props.text)

async function submit(): Promise<void> {
  if (!changed.value || tooLong.value || status.value.kind === 'saving') {
    return
  }
  status.value = { kind: 'saving' }
  try {
    await props.save(draft.value)
    status.value = { kind: 'idle' }
  } catch (error) {
    status.value = { kind: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}
</script>

<template>
  <SettingsPage
    title="Instructions"
    description="What Demi follows in every conversation, together with each project’s AGENTS.md or CLAUDE.md."
  >
    <SettingsGroup
      title="Personal instructions"
      description="For example, the language to reply in or how to write commit messages. Empty to have none."
    >
      <template #actions>
        <Button
          size="sm"
          variant="primary"
          :disabled="!changed || tooLong"
          :loading="status.kind === 'saving'"
          @click="submit"
        >
          Save
        </Button>
      </template>
      <div class="flex flex-col gap-2 p-3">
        <TextArea
          v-model="draft"
          :rows="12"
          placeholder="Reply in Chinese. Write commit messages in English."
          aria-label="Personal instructions"
          @keydown.meta.enter.prevent="submit"
          @keydown.ctrl.enter.prevent="submit"
        />
        <div class="flex items-start justify-between gap-3 text-[12px] leading-4">
          <span v-if="status.kind === 'failed'" class="text-on-danger">{{ status.message }}</span>
          <span v-else />
          <span class="shrink-0 tabular-nums" :class="tooLong ? 'text-on-danger' : 'text-fg-subtle'">
            {{ length.toLocaleString('en-US') }} / {{ LIMIT.toLocaleString('en-US') }}
          </span>
        </div>
      </div>
    </SettingsGroup>
  </SettingsPage>
</template>
