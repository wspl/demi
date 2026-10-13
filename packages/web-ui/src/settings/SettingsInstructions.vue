<script setup lang="ts">
import RequestIdLine from '../ui/RequestIdLine.vue'
import { shownError, type ShownError } from '../infra/errors'
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
/** The refusal of the last save, until the next one starts; Save shows the work itself. */
const failure = ref<ShownError | null>(null)

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

const saveButton = ref<InstanceType<typeof Button> | null>(null)

async function submit(): Promise<void> {
  failure.value = null
  try {
    await props.save(draft.value)
  } catch (error) {
    failure.value = shownError(error)
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
          ref="saveButton"
          :operation="{ action: 'save', target: 'instructions' }"
          :disabled="!changed || tooLong"
          @click="submit"
        >
          Save
        </Button>
      </template>
      <div class="flex flex-col gap-2 p-3">
        <TextArea
          v-model="draft"
          :rows="12"
          aria-label="Personal instructions"
          @keydown.meta.enter.prevent="saveButton?.press()"
          @keydown.ctrl.enter.prevent="saveButton?.press()"
        />
        <div class="flex items-start justify-between gap-3 text-[12px] leading-4">
          <div v-if="failure" class="min-w-0">
            <p class="text-on-danger">{{ failure.message }}</p>
            <RequestIdLine v-if="failure.requestId" class="text-fg-muted" :request-id="failure.requestId" />
          </div>
          <span v-else />
          <span class="shrink-0 tabular-nums" :class="tooLong ? 'text-on-danger' : 'text-fg-subtle'">
            {{ length.toLocaleString('en-US') }} / {{ LIMIT.toLocaleString('en-US') }}
          </span>
        </div>
      </div>
    </SettingsGroup>
  </SettingsPage>
</template>
