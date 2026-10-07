<script setup lang="ts">
import { ref } from 'vue'
import { ArrowLeft, ArrowRight, RotateCw, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { SentenceText } from '../ui/ui-text'

/**
 * The address bar both kinds of web page tab share
 * (`web-application.md` § Package responsibilities): a `browser` tab of the
 * conversation browser has its own history, a `page` tab keeps a framed
 * page's history to itself. It shows the page's address and follows it,
 * except while the user has the field: then it keeps what the user types,
 * and leaving without Enter shows the page's address again. While the page
 * loads, Reload is Stop, in the same place, as in a web browser.
 */
const props = withDefaults(
  defineProps<{
    /** The page's address. */
    address: string
    /** Why Back is unavailable, when it is, such as a history with no page before this one. */
    backReason?: SentenceText | null
    /** Why Forward is unavailable, when it is. */
    forwardReason?: SentenceText | null
    canReload?: boolean
    /** The page loads: Reload is Stop meanwhile. */
    loading?: boolean
    /** Why Stop is unavailable while the page loads, such as a tab still opening. */
    stopReason?: SentenceText | null
    /** Take focus on mount, the address selected: a new tab waits for where to go. */
    focused?: boolean
  }>(),
  { backReason: null, forwardReason: null, canReload: true, loading: false, stopReason: null, focused: false },
)

const emit = defineEmits<{
  /** Enter on an address, as a whole URL: what the user typed, `https://` before it when it names no scheme. */
  submit: [url: string]
  back: []
  forward: []
  reload: []
  stop: []
}>()

const field = ref<InstanceType<typeof TextInput> | null>(null)
/** What the field holds while the user has it; null while it shows the page's address. */
const draft = ref<string | null>(null)
/** The pointer that is giving the field its focus; its release must not drop the selection the focus made. */
let focusing = false

// A click into the address selects it whole, as a web browser's does: the next thing typed replaces it.
function pointerDown(event: PointerEvent): void {
  focusing = event.target !== document.activeElement
}

function pointerUp(event: MouseEvent): void {
  if (!focusing) {
    return
  }
  focusing = false
  // The release would place a caret and end the selection.
  event.preventDefault()
  field.value?.select()
}

// The page's address stands still while the user has the field, selected whole as a web browser's is.
function focus(): void {
  draft.value = props.address
  field.value?.select()
}

/** The URL `text` names, `https://` before it when it names no scheme; null when it names none. */
function urlOf(text: string): string | null {
  const typed = text.trim()
  const candidate = typed.includes('://') ? typed : `https://${typed}`
  if (!typed || !URL.canParse(candidate)) {
    return null
  }
  return new URL(candidate).href
}

// An address is submitted and the field lets go, so keys reach the page again. Text that names no
// address stays in the field for the user to correct.
function submit(): void {
  const url = urlOf(draft.value ?? props.address)
  if (url === null) {
    return
  }
  emit('submit', url)
  field.value?.el?.blur()
}
</script>

<template>
  <div class="flex h-11 shrink-0 items-center gap-1 px-2">
    <div class="flex shrink-0 items-center">
      <IconButton
        :icon="ArrowLeft"
        variant="ghost"
        aria-label="Back"
        :disabled="backReason !== null"
        :disabled-reason="backReason ?? undefined"
        @click="emit('back')"
      />
      <IconButton
        :icon="ArrowRight"
        variant="ghost"
        aria-label="Forward"
        :disabled="forwardReason !== null"
        :disabled-reason="forwardReason ?? undefined"
        @click="emit('forward')"
      />
      <!-- Reload and Stop share the place and the size, and a click on either shows the other at once:
           Reload turns into Stop rather than turning its icon. -->
      <Tooltip v-if="loading" content="Stop" :disabled="stopReason !== null" class="inline-flex">
        <IconButton
          :icon="X"
          variant="ghost"
          aria-label="Stop"
          :disabled="stopReason !== null"
          :disabled-reason="stopReason ?? undefined"
          @click="emit('stop')"
        />
      </Tooltip>
      <Tooltip v-else content="Reload" :disabled="!canReload" class="inline-flex">
        <IconButton
          :icon="RotateCw"
          variant="ghost"
          aria-label="Reload"
          :disabled="!canReload"
          @click="emit('reload')"
        />
      </Tooltip>
    </div>
    <TextInput
      ref="field"
      class="ml-2 min-w-0 flex-1"
      :focused="focused"
      :model-value="draft ?? address"
      placeholder="Enter address"
      aria-label="Browser address"
      @update:model-value="draft = $event"
      @keydown.enter="submit"
      @focus="focus"
      @blur="draft = null"
      @pointerdown="pointerDown"
      @mouseup="pointerUp"
    />
    <!-- A trailing control stands as far from the address as the navigation group does. -->
    <div v-if="$slots.trailing" class="ml-2 flex shrink-0 items-center">
      <slot name="trailing" />
    </div>
  </div>
</template>
