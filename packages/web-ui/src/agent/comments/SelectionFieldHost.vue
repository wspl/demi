<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { autoUpdate, flip, offset, shift, useFloating } from '@floating-ui/vue'
import { stackAbove } from '../../overlay/stacking'
import { usePhone } from '../../ui/phone'
import { shortcutSet } from '../../ui/shortcut'
import {
  selectedLinesBounds,
  selectionFieldOffer,
  shownContextMenu,
  type SelectionFieldActions,
  type SelectionFieldOffer,
} from '../../ui/context-menu'
import { heldSelection } from '../../ui/drawn-selection'
import SelectionField from './SelectionField.vue'
import { chosenWay, closePictureField, pictureField, type FieldWay } from './selection-field'

/**
 * The page's one comment field, which the app mounts once at its root. It
 * stands by what is open for it:
 *
 * - the page's selection, while it holds one object's own text and the object
 *   offers a field for it (`selectionField`)
 * - a picture a menu's Comment or Ask opened it on (`pictureField`)
 *
 * It is centered under the selected lines, or the picture, once the mouse that
 * drags the selection is released, and above them where there is no room
 * under them. A phone opens no field: a selection there is the system's.
 *
 * The field leaves the selection to the browser: its words are typed in a
 * frame of their own, so the focus in it leaves the page's selection where it
 * is, and every press, click and key on the page does
 * what it does with no field. The field follows the selection: one extended or
 * shrunk from the same start or end keeps it, another opens a new one, and
 * none closes it and drops what was typed. It takes the focus as it opens, so
 * a key typed after the selection, an input method's composition included,
 * starts in it, though not while a button is down, which may still be making
 * the selection.
 *
 * Escape closes it and leaves the selection, which opens no field again until
 * the next press.
 *
 * It steps aside while a context menu is open, the one control for the selection
 * then, and comes back as the menu closes, with what was typed.
 *
 * What an action does is the object's, which declares it: the field says it with
 * the words typed, and closes once it is done.
 */
const phone = usePhone()
const selection = shallowRef<SelectionFieldOffer | null>(null)
/** Bumped for each selection or picture the field opens for, so it starts empty. */
const opening = ref(0)
/** The action Return presses. */
const way = ref<FieldWay>('comment')
/**
 * The field waits for the button to come up: a mouse drags the selection, or a
 * press, as a double click's second, made the selection the field opened for.
 */
const waiting = ref(false)
/** A button is down on the page, where it may still be making the selection: the field takes no focus until it is up. */
const pressing = ref(false)
/** Where the button went down, to tell a drag from a click. */
let pressedAt: { x: number; y: number } | null = null
/** The field had the focus when a menu opened, which it takes back as the menu closes. */
const focused = ref(false)
/** An action is under way. */
const busy = ref(false)
/** The field takes the focus once it is drawn. */
const focusWanted = ref(false)
/** The range the user closed the field on, which does not open it again until the next press. */
let dismissed: Range | null = null

const floating = ref<HTMLElement>()
const box = ref<InstanceType<typeof SelectionField>>()
const text = ref('')

/** What the field stands by: a picture a menu chose, or else the selection. */
const current = computed<{ actions: SelectionFieldActions; element: Element; rect: () => DOMRect } | null>(() => {
  const picture = pictureField.value
  if (picture)
    return { actions: picture.actions, element: picture.image, rect: () => picture.image.getBoundingClientRect() }
  const now = selection.value
  if (now)
    return { actions: now.actions, element: now.object, rect: () => selectedLinesBounds(now.range) ?? now.range.getBoundingClientRect() }
  return null
})

function sameEnd(a: Range, b: Range): boolean {
  return (a.startContainer === b.startContainer && a.startOffset === b.startOffset) ||
    (a.endContainer === b.endContainer && a.endOffset === b.endOffset)
}

function open(): void {
  opening.value += 1
  waiting.value ||= pressing.value
  way.value = pictureField.value?.way ?? 'comment'
  text.value = ''
  focused.value = false
  busy.value = false
  focusWanted.value = true
}

/** Reads the page's selection, which the field follows. */
function read(): void {
  // A phone's selection is the system's alone.
  const now = phone.value ? null : selectionFieldOffer()
  const held = selection.value
  if (!now) {
    if (held)
      closeAll()
    selection.value = null
    // Nothing is selected: the same words selected again are new.
    dismissed = null
    return
  }
  if (held && sameEnd(held.range, now.range)) {
    // Extended or shrunk, or the same words again: the same field follows.
    selection.value = now
    return
  }
  if (dismissed && sameEnd(dismissed, now.range))
    return
  dismissed = null
  closePictureField()
  selection.value = now
  open()
}

watch(pictureField, (now) => {
  if (now) {
    selection.value = null
    open()
  }
})

function closeAll(): void {
  if (selection.value)
    dismissed = selection.value.range.cloneRange()
  selection.value = null
  closePictureField()
  focused.value = false
  busy.value = false
}

async function done(action: 'comment' | 'ask' | 'send', words: string): Promise<void> {
  const taken = current.value
  if (!taken || busy.value)
    return
  const at = opening.value
  const result = taken.actions[action]?.(words)
  if (result) {
    busy.value = true
    try {
      await result
    } catch {
      // The object said why, and the field keeps its words to try again.
      return
    } finally {
      busy.value = false
    }
  }
  // Closed or opened elsewhere meanwhile: what it held is gone.
  if (opening.value !== at || !current.value)
    return
  window.getSelection()?.removeAllRanges()
  dismissed = null
  selection.value = null
  closePictureField()
  focused.value = false
}

const listeners = new AbortController()
onMounted(() => {
  const { signal } = listeners
  document.addEventListener('selectionchange', () => read(), { signal })
  document.addEventListener('pointerdown', (event) => {
    // The menu is the one control for the selection while it is open.
    if (shownContextMenu.value !== null)
      return
    const inside = floating.value?.contains(event.target as Node) ?? false
    // A right-click, or a control-click on a Mac, opens a menu and presses nothing.
    if (event.button === 2 || (event.button === 0 && event.ctrlKey && shortcutSet() === 'mac')) {
      focused.value = !inside && (box.value?.focused ?? false)
      return
    }
    if (event.button !== 0 || inside)
      return
    // A press starts a selection of its own: even the words the field closed on are new.
    dismissed = null
    pressing.value = true
    pressedAt = { x: event.clientX, y: event.clientY }
    // A picture's field stands by no selection, which the press would end.
    if (!selection.value)
      closeAll()
  }, { capture: true, signal })
  document.addEventListener('pointermove', (event) => {
    // The pointer moved this far, which the system takes for a drag: the field stands aside until it ends.
    if (!pressedAt || waiting.value || event.buttons === 0 || Math.hypot(event.clientX - pressedAt.x, event.clientY - pressedAt.y) < 4)
      return
    waiting.value = event.pointerType === 'mouse'
  }, { capture: true, signal })
  // The press ends with its release, or without one when the system takes it over, as for dragging text, or the window loses it.
  const endPress = () => {
    waiting.value = false
    pressing.value = false
    pressedAt = null
  }
  document.addEventListener('pointerup', endPress, { capture: true, signal })
  document.addEventListener('pointercancel', endPress, { capture: true, signal })
  window.addEventListener('blur', endPress, { signal })
})
onBeforeUnmount(() => {
  listeners.abort()
  heldSelection.value = null
})

// A picture a menu opened the field on, which the browser does not select, is drawn as selected.
watch(pictureField, (picture) => {
  if (picture) {
    const range = document.createRange()
    range.selectNode(picture.image)
    heldSelection.value = range
  } else
    heldSelection.value = null
})

// A reply that renders its text again collapses the selection in Chrome without a
// `selectionchange`: the change of the text is read as the selection's, so the
// field never stands for text that went.
watch(selection, (now, _before, onCleanup) => {
  if (!now)
    return
  const observer = new MutationObserver(() => read())
  observer.observe(now.object, { childList: true, subtree: true, characterData: true })
  onCleanup(() => observer.disconnect())
})

// A menu's Comment or Ask chose the way for the field the selection has.
const menuOpen = computed(() => shownContextMenu.value !== null)
watch([chosenWay, menuOpen], ([chosen, menu]) => {
  if (!chosen || menu)
    return
  // The words the field was closed on, still selected, open it again for the menu's choice.
  if (!current.value) {
    dismissed = null
    read()
  }
  if (current.value) {
    way.value = chosen.way
    focusWanted.value = true
  }
  chosenWay.value = null
}, { flush: 'post' })

// The focus the field gave up for a menu returns as the menu closes.
watch(menuOpen, (menu) => {
  if (!menu && focused.value && current.value)
    focusWanted.value = true
})

// A page that turns a phone's closes the field: a selection there is the system's.
watch(phone, (now) => {
  if (now)
    closeAll()
})

const shown = computed(() => current.value !== null && !waiting.value && !phone.value)

const reference = computed(() => {
  const now = current.value
  if (!now)
    return null
  return { contextElement: now.element, getBoundingClientRect: now.rect }
})

const { floatingStyles, isPositioned } = useFloating(reference, floating, {
  open: shown,
  placement: 'bottom',
  strategy: 'fixed',
  middleware: [offset(8), flip({ padding: 8 }), shift({ padding: 8 })],
  whileElementsMounted: autoUpdate,
})
const z = stackAbove()

// A field drawn hidden, as before it is placed or while a menu is open, takes no focus.
watch([focusWanted, box, menuOpen, () => isPositioned.value, pressing], async ([wanted, field, menu, drawn, down]) => {
  if (!wanted || !field || menu || !drawn || down)
    return
  await nextTick()
  // The click that ended the selection may focus what holds its end, a side question's card, as it finishes: the field takes the focus after it.
  await new Promise(requestAnimationFrame)
  box.value?.focus()
  focusWanted.value = false
}, { flush: 'post' })

const placement = computed(() => ({ ...floatingStyles.value, visibility: isPositioned.value ? undefined : 'hidden' } as const))

/** What the field takes, wherever it stands. */
const fieldBindings = computed(() => ({
  'modelValue': text.value,
  'onUpdate:modelValue': (words: string) => (text.value = words),
  'ask': !!current.value?.actions.ask,
  'send': !!current.value?.actions.send,
  'way': way.value,
  'unavailable': current.value?.actions.unavailable,
  'busy': busy.value,
  'onComment': (words: string) => done('comment', words),
  'onAsk': (words: string) => done('ask', words),
  'onSend': (words: string) => done('send', words),
  'pageKeys': !pictureField.value && selection.value !== null,
  'onClose': closeAll,
}))

</script>

<template>
  <Teleport to="body">
    <div
      v-if="shown && current"
      ref="floating"
      data-selection-field-layer
      :style="{ ...placement, zIndex: z(), ...(menuOpen ? { visibility: 'hidden' } : {}) }"
    >
      <SelectionField :key="opening" ref="box" v-bind="fieldBindings" />
    </div>
  </Teleport>
</template>
