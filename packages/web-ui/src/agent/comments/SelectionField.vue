<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { ArrowUp, MessageCircleMore, MessageCircleQuestionMark } from '@lucide/vue'
import FramedField from '../../ui/FramedField.vue'
import IconButton from '../../ui/IconButton.vue'
import MessageField from '../message-editor/MessageField.vue'
import { saysSomething, type DraftMessage, type MessageFiles } from './comments'
import { answerForPage } from '../../ui/page-keys'
import { withCommandKey } from '../../ui/shortcut'
import Tooltip from '../../ui/Tooltip.vue'
import type { SentenceText } from '../../ui/ui-text'
import { t } from '../locales/text'
import { otherWayKeys } from '../send-way'
import type { FieldWay } from './selection-field'

/**
 * The one field a remark on a part of a reply is written in: a capsule of one
 * line holding the Add Files button, the message, a gray round button and a
 * blue one. The message is written as the composer's is (`MessageField.vue`):
 * its Markdown kept as typed and highlighted, its files pasted, dropped or
 * added as the composer takes them.
 *
 * - The blue button is the chosen way, Comment unless a menu's Ask chose it,
 *   and Return presses it; the gray one is the other way. An Ask the page does
 *   not offer leaves the blue Comment alone.
 * - While the set's command key is held, ⌘ on a Mac and Control elsewhere, the
 *   blue button sends the quote and the words to the main agent now, and
 *   ⌘Return (Control-Return) does, where the page offers Send. Its tooltip
 *   names that key, as the message field's send button names its other way.
 * - Shift-Return starts a new line. A message that outgrows the line, or
 *   holds a new line, opens the capsule into a box: the message above, the
 *   buttons under it, as the composer does.
 * - With no words and no file, Return and the buttons shake the field once.
 * - Escape closes it (`close`); its owner also closes it when the selection
 *   it stands for is gone.
 *
 * The owner takes the focus into it (`focus`), so a key typed after the
 * selection, an input method's composition included, starts in the field. The
 * message is written in a frame of its own (`FramedField.vue`), so the
 * focus there leaves the page's selection the browser's. Until a field that
 * stands for that selection (`pageKeys`) holds a word or a file, the keys the
 * page answers act on the page (`answerForPage`), and Copy copies the page's
 * selection while the field selects none of its own.
 */
const props = defineProps<{
  /** The files of the conversation the field's message is written in. */
  files: MessageFiles
  /** Whether the page offers Ask; without it the field has no Ask button. */
  ask?: boolean
  /** Whether the page offers Send, which the held command key turns the blue button into. */
  send?: boolean
  /** The action Return presses. */
  way?: FieldWay
  /** Why no action can act now, as the buttons' tooltip, such as a reply still being written. */
  unavailable?: SentenceText | null
  /** An action is under way, as a picture being kept: nothing acts meanwhile. */
  busy?: boolean
  /** The command key, pinned as held, for a gallery's specimen; the field follows the keyboard otherwise. */
  held?: boolean
  /** The field stands for the page's selection, which the keys the page answers act on while it holds no words; not for a picture. */
  pageKeys?: boolean
}>()

const message = defineModel<DraftMessage>({ default: () => ({ text: '', fileIds: [] }) })

const emit = defineEmits<{
  comment: [message: DraftMessage]
  ask: [message: DraftMessage]
  send: [message: DraftMessage]
  close: []
}>()

const root = ref<HTMLElement>()
const framed = ref<InstanceType<typeof FramedField>>()
const field = ref<InstanceType<typeof MessageField>>()
const buttons = ref<HTMLElement>()
const shaking = ref(false)
const empty = computed(() => !saysSomething(message.value))
/** The message needs more than the line the buttons leave it. */
const open = ref(false)
const blocked = computed(() => !!props.unavailable || props.busy === true)
/** The blue button's way; the gray one is the other. */
const chosen = computed<FieldWay>(() => props.way === 'ask' && props.ask ? 'ask' : 'comment')

/** The set's command key is down while the field has the focus. */
const commandDown = ref(false)
const sending = computed(() => !!props.send && (props.held || commandDown.value))

function track(event: KeyboardEvent): void {
  commandDown.value = withCommandKey(event)
}
function release(): void {
  commandDown.value = false
}
onMounted(() => {
  window.addEventListener('keydown', track, true)
  window.addEventListener('keyup', track, true)
  window.addEventListener('blur', release)
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', track, true)
  window.removeEventListener('keyup', track, true)
  window.removeEventListener('blur', release)
})

const { width } = useElementSize(root)
/**
 * How wide the message's line is in the one-line shape: the field's width less
 * the buttons', which stand in the field either way, so the shape does not
 * flip as it changes.
 */
const lineWidth = computed(() => {
  void width.value
  const box = root.value
  if (!box || !buttons.value)
    return 0
  const style = getComputedStyle(box)
  return box.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)
    - buttons.value.offsetWidth - (parseFloat(style.columnGap) || 0)
})

defineExpose({
  /** Takes the focus, the caret at the end of what is written. */
  focus(): void {
    framed.value?.focusFrame()
    field.value?.takeFocus()
  },
  /** Whether the focus is in the field. */
  get focused(): boolean {
    return framed.value?.focused ?? false
  },
})

async function shake(): Promise<void> {
  shaking.value = false
  await nextTick()
  // A frame between, so a second press restarts the animation.
  requestAnimationFrame(() => (shaking.value = true))
}

function act(way: 'comment' | 'ask' | 'send'): void {
  if (blocked.value)
    return
  if (empty.value) {
    void shake()
    return
  }
  const written = { text: message.value.text.trim(), fileIds: [...message.value.fileIds] }
  if (way === 'comment')
    emit('comment', written)
  else if (way === 'ask')
    emit('ask', written)
  else
    emit('send', written)
}

/** Return, or ⌘Return (Control-Return) for the other way: Send where the page offers it. */
function submit(otherWay: boolean): void {
  if (otherWay || sending.value) {
    if (props.send)
      act('send')
  } else
    act(chosen.value)
}

/** The blue button: the chosen way, or Send while the command key is held. */
function primary(event?: MouseEvent): void {
  act(props.send && (sending.value || (event && withCommandKey(event))) ? 'send' : chosen.value)
}

/**
 * The keys that reach the field before its editor: the command key's state,
 * Copy of the page's selection while the field selects nothing of its own,
 * and, while it holds nothing, the keys the page answers.
 */
function onKeydown(event: KeyboardEvent): void {
  // The frame's keys reach no listener of the page's window.
  track(event)
  // Safari ends a composition with an Enter whose `isComposing` is already false
  // and whose key code is 229: it confirms the words, and sends nothing.
  if (event.isComposing || event.keyCode === 229)
    return
  const own = framed.value?.document?.getSelection()
  if ((!own || own.isCollapsed) && withCommandKey(event) && event.key.toLowerCase() === 'c' && !event.shiftKey && !event.altKey) {
    // Copy with nothing of the field's selected copies the page's selection, as the page would.
    if (!window.getSelection()?.isCollapsed) {
      event.preventDefault()
      event.stopPropagation()
      document.execCommand('copy')
    }
    return
  }
  if (props.pageKeys && empty.value && root.value && answerForPage(event, root.value)) {
    event.preventDefault()
    event.stopPropagation()
  }
}

/** A press on the buttons keeps the focus where it is. */
function keep(event: Event): void {
  event.preventDefault()
}

const label = (way: FieldWay) => way === 'ask' ? t('ask.ask') : t('comments.comment')
const other = computed<FieldWay>(() => chosen.value === 'ask' ? 'comment' : 'ask')
const primaryLabel = computed(() => sending.value ? t('composer.send') : label(chosen.value))
/** As the message field's send button: the way, and the key that sends instead. */
const primaryTip = computed(() => props.send && !sending.value
  ? t('composer.sendOtherWay', { way: label(chosen.value), keys: otherWayKeys(), other: t('composer.send').toLowerCase() })
  : primaryLabel.value)
const primaryIcon = computed(() => sending.value ? ArrowUp : chosen.value === 'ask' ? MessageCircleQuestionMark : MessageCircleMore)
const otherIcon = computed(() => other.value === 'ask' ? MessageCircleQuestionMark : MessageCircleMore)
</script>

<template>
  <div
    ref="root"
    data-selection-field
    class="selection-field overlay-shell grid w-[400px] max-w-[calc(100vw-1rem)] items-center gap-x-1.5 py-1.5 pl-3.5 pr-1.5"
    :class="[open ? 'selection-field-open gap-y-1 rounded-[20px] pt-2' : 'rounded-full', { 'selection-field-shake': shaking }]"
    @animationend="shaking = false"
  >
    <div class="relative min-w-0 [grid-area:words]">
      <FramedField ref="framed" v-slot="{ document: frameDocument }">
        <div
          class="max-h-32 overflow-y-auto py-1 text-chrome leading-5 text-fg"
          @keydown.capture="onKeydown"
          @keyup="track"
        >
          <MessageField
            ref="field"
            v-model="message"
            v-model:multiline="open"
            :files="files"
            :document="frameDocument"
            :line-width="lineWidth"
            :placeholder="t('comments.placeholder')"
            :label="t('comments.placeholder')"
            cancelable
            @submit="submit"
            @cancel="emit('close')"
            @blur="release"
          />
        </div>
      </FramedField>
    </div>
    <div ref="buttons" class="flex items-center gap-1.5 justify-self-end [grid-area:buttons]">
      <Tooltip v-if="ask" :content="blocked ? unavailable ?? label(other) : label(other)">
        <IconButton
          :icon="otherIcon"
          variant="ghost"
          circle
          :disabled="blocked"
          :disabled-reason="unavailable ?? undefined"
          :aria-label="label(other)"
          @mousedown="keep"
          @click="act(other)"
        />
      </Tooltip>
      <Tooltip :content="blocked ? unavailable ?? primaryTip : primaryTip">
        <IconButton
          :icon="primaryIcon"
          variant="accent"
          circle
          data-selection-field-primary
          :disabled="blocked"
          :disabled-reason="unavailable ?? undefined"
          :aria-label="primaryLabel"
          @mousedown="keep"
          @click="primary($event)"
        />
      </Tooltip>
    </div>
  </div>
</template>

<style scoped>
.selection-field {
  grid-template-columns: minmax(0, 1fr) auto;
  grid-template-areas: "words buttons";
}

.selection-field-open {
  grid-template-areas: "words words" ". buttons";
}

.selection-field-shake {
  animation: selection-field-shake 0.32s ease-out;
}

@keyframes selection-field-shake {
  15%, 55% {
    transform: translateX(-5px);
  }
  35%, 75% {
    transform: translateX(5px);
  }
}

@media (prefers-reduced-motion: reduce) {
  /* Without motion the refusal is a brief dip of the field's opacity. */
  .selection-field-shake {
    animation: selection-field-dip 0.32s ease-out;
  }

  @keyframes selection-field-dip {
    50% {
      opacity: 0.5;
    }
  }
}
</style>
