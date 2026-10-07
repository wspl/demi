<script setup lang="ts">
import { ref } from 'vue'
import { ArrowLeft, ArrowRight, RotateCw } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import TextInput from '../ui/TextInput.vue'

/**
 * The address bar both kinds of web page tab share
 * (`web-application.md` § Package responsibilities): a `browser` tab of the
 * conversation browser has its own history, a `page` tab keeps a framed
 * page's history to itself. It shows the page's address and follows it,
 * except while the user has the field: then it keeps what the user types,
 * and leaving without Enter shows the page's address again.
 */
const props = withDefaults(
  defineProps<{
    /** The page's address. */
    address: string
    /** Why Back and Forward are unavailable, when they are. */
    historyReason?: string | null
    canReload?: boolean
    /** Take focus on mount, the address selected: a new tab waits for where to go. */
    focused?: boolean
  }>(),
  { historyReason: null, canReload: true, focused: false },
)

const emit = defineEmits<{
  /** Enter on an address, as a whole URL: what the user typed, `https://` before it when it names no scheme. */
  submit: [url: string]
  back: []
  forward: []
  reload: []
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
        :disabled="historyReason !== null"
        :disabled-reason="historyReason ?? undefined"
        @click="emit('back')"
      />
      <IconButton
        :icon="ArrowRight"
        variant="ghost"
        aria-label="Forward"
        :disabled="historyReason !== null"
        :disabled-reason="historyReason ?? undefined"
        @click="emit('forward')"
      />
      <IconButton
        :icon="RotateCw"
        variant="ghost"
        aria-label="Refresh"
        spin-on-click
        :disabled="!canReload"
        @click="emit('reload')"
      />
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
