<script setup lang="ts">
import { nextTick, ref } from 'vue'
import { ArrowLeft, ArrowRight, RotateCw, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { provideTooltipPlacement } from '../ui/tooltip-placement'
import type { SentenceText } from '../ui/ui-text'
import { addressUrl } from './address'

/**
 * The address bar both kinds of web page tab share
 * (`web-application.md` § Package responsibilities): a `browser` tab of the
 * conversation browser has its own history, a `page` tab keeps a framed
 * page's history to itself. It shows the page's address and follows it,
 * except while the user has the field: then it keeps what the user types,
 * and leaving without Enter, or Escape, shows the page's address again. It
 * reads what the user typed as a browser's address bar does (`addressUrl`).
 * While the page
 * loads, Reload is Stop, in the same place, as in a web browser; Stop is
 * never unavailable while the page loads. It stands right under a tab
 * strip, so the tips of its buttons, and of those its caller adds, open
 * below them, never over the strip.
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
    /** Take focus on mount, the address selected: a new tab waits for where to go. */
    focused?: boolean
  }>(),
  { backReason: null, forwardReason: null, canReload: true, loading: false, focused: false },
)

const emit = defineEmits<{
  /** Enter on an address, as the whole URL a browser's address bar opens for what the user typed. */
  submit: [url: string]
  back: []
  forward: []
  reload: []
  stop: []
}>()

provideTooltipPlacement('bottom')

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

// An address is submitted and the field lets go, so keys reach the page again. An empty field opens
// nothing.
function submit(): void {
  const url = addressUrl(draft.value ?? props.address)
  if (url === null) {
    return
  }
  emit('submit', url)
  field.value?.el?.blur()
}

// Escape gives up what the user typed: the page's address shows again, selected, as in a browser.
async function revert(): Promise<void> {
  draft.value = props.address
  // Selected once the field shows the address: writing its value puts the caret at the end.
  await nextTick()
  field.value?.select()
}

defineExpose({
  /** Puts the focus in the address, selected whole, as a browser's ⌘L does. */
  focus(): void {
    field.value?.focus()
    field.value?.select()
  },
})
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
      <Tooltip v-if="loading" content="Stop" class="inline-flex">
        <IconButton :icon="X" variant="ghost" aria-label="Stop" @click="emit('stop')" />
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
      @keydown.esc.stop.prevent="revert"
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
