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
import { rangeHas } from './anchors'
import { heldSelection } from '../../ui/drawn-selection'
import SelectionField from './SelectionField.vue'
import { chosenWay, closePictureField, pictureField, type FieldWay } from './selection-field'

/**
 * The page's one comment field, which the app mounts once at its root
 * (`docs/decisions/0092-a-remark-stands-under-its-line.md`). It stands by
 * what is open for it:
 *
 * - the page's selection, while it holds one object's own text and the object
 *   offers a field for it (`selectionField`)
 * - a picture a menu's Comment or Ask opened it on (`pictureField`)
 *
 * It is centered under the selected lines, or the picture, once the mouse that
 * drags the selection is released, and above them where there is no room
 * under them. A phone opens no field: a selection there is the system's.
 *
 * The field takes the focus as it opens, so a key typed after the selection,
 * an input method's composition included, starts in it: an input method works
 * only in a focused field, and a key that arrives with the focus elsewhere has
 * already been taken as a letter. The focus in a field ends the selection on
 * the page, so the selection is kept for it: its words are drawn as selected
 * (`heldSelection`) and Cmd-C copies them.
 *
 * A right-click on the selected words puts the selection back on the page while
 * their menu is open, which reads it, and the field takes the focus again as the
 * menu closes.
 *
 * Closing: Escape, a click elsewhere and another selection close it and drop
 * what was typed. Another selection is one that keeps neither end of the one the
 * field opened for. A press on the page decides nothing until it is released, so
 * the field stays as it is while words are clicked again and again: a click on
 * the selected words leaves it, and a click that selects the same words or
 * extends them leaves it in place, as a double or triple click does; one that
 * clears the selection elsewhere closes it, and a drag stands it aside until it
 * ends.
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
/** A mouse holds the selection it drags: the field waits for its release. */
const dragging = ref(false)
/** What a press acts on instead of the text under it: a link, a button, a picture or other media. */
const ACTS = 'a, button, [role="button"], img, video, audio, canvas, svg'
/**
 * A press on the page, from its start until it is released: it decides the
 * selection then. `onText` is whether it is on the words themselves, not on what
 * acts when it is pressed.
 */
let press: { x: number; y: number; dragged: boolean; onText: boolean } | null = null
/** The field has had the focus, which ended the selection on the page until the field closes. */
const focused = ref(false)
/** An action is under way. */
const busy = ref(false)
/** The field takes the focus once it is drawn. */
const focusWanted = ref(false)
/** The range the user closed the field on, which does not open it again. */
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
  way.value = pictureField.value?.way ?? 'comment'
  text.value = ''
  focused.value = false
  busy.value = false
  focusWanted.value = true
}

/**
 * Reads the page's selection. `released` is where a press on the page ended and
 * whether it was on the words, which took the focus from the field that held it:
 * that press is what the selection is read for, however many clicks make it.
 */
function read(released?: { x: number; y: number; onText: boolean }): void {
  // The press is still deciding the selection: a click collapses it for a moment
  // before its second click selects the words, and a drag changes it by the pixel.
  if (press)
    return
  // The selection is the field's own text then.
  if (focused.value && !released)
    return
  // A phone's selection is the system's alone.
  const now = phone.value ? null : selectionFieldOffer()
  const held = selection.value
  if (!now) {
    if (held && released) {
      // A click on the selected words is none on the page: the field stays. One on a link, a button or a picture is a click elsewhere, which acts.
      if (released.onText && rangeHas(held.range, released.x, released.y)) {
        focusWanted.value = true
        return
      }
      closeAll()
    }
    selection.value = null
    // Nothing is selected: the same words selected again are new.
    dismissed = null
    return
  }
  if (held && sameEnd(held.range, now.range)) {
    // Extended or shrunk, or the same words again: the same field follows.
    selection.value = now
    if (released)
      focusWanted.value = true
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

/** Puts the selection the field stands by back on the page, where a menu reads it. */
function restoreSelection(): void {
  const held = selection.value
  if (!held)
    return
  ;(document.activeElement as HTMLElement | null)?.blur()
  const { range } = held
  window.getSelection()?.setBaseAndExtent(range.startContainer, range.startOffset, range.endContainer, range.endOffset)
}

const listeners = new AbortController()
onMounted(() => {
  const { signal } = listeners
  document.addEventListener('selectionchange', () => read(), { signal })
  document.addEventListener('pointerdown', (event) => {
    // The menu is the one control for the selection while it is open.
    if (shownContextMenu.value !== null)
      return
    const target = event.target as Node
    const inside = floating.value?.contains(target) ?? false
    // A right-click, or a control-click on a Mac, opens a menu and presses nothing.
    if (event.button === 2 || (event.button === 0 && event.ctrlKey && shortcutSet() === 'mac')) {
      if (!inside && focused.value)
        restoreSelection()
      return
    }
    if (event.button !== 0 || inside)
      return
    // A press starts a selection of its own: even the same words are new. The
    // field it leaves stands until the press is released, so that the clicks of a
    // double click do not close and open it again.
    dismissed = null
    press = { x: event.clientX, y: event.clientY, dragged: false, onText: !(target instanceof Element ? target : target.parentElement)?.closest(ACTS) }
    if (!selection.value)
      closeAll()
  }, { capture: true, signal })
  document.addEventListener('pointermove', (event) => {
    // The pointer moved this far, which the system takes for a drag: it selects words of its own.
    if (!press || press.dragged || event.buttons === 0 || Math.hypot(event.clientX - press.x, event.clientY - press.y) < 4)
      return
    press.dragged = true
    dragging.value = event.pointerType === 'mouse'
    closeAll()
    dismissed = null
  }, { capture: true, signal })
  document.addEventListener('pointerup', (event) => {
    dragging.value = false
    const released = press ? { x: event.clientX, y: event.clientY, onText: press.onText } : undefined
    press = null
    read(released)
  }, { capture: true, signal })
  // The press ends without a release when the system takes it over, as for dragging text, or the window loses it.
  const endPress = () => {
    dragging.value = false
    if (press) {
      press = null
      read()
    }
  }
  document.addEventListener('pointercancel', endPress, { capture: true, signal })
  window.addEventListener('blur', endPress, { signal })
  // The focus in the field ended the selection on the page, so Copy copies what the field holds for it.
  document.addEventListener('copy', (event) => {
    const held = selection.value
    const field = box.value?.focused ? document.activeElement : null
    if (!held || !(field instanceof HTMLTextAreaElement) || field.selectionStart !== field.selectionEnd)
      return
    event.clipboardData?.setData('text/plain', held.text)
    event.preventDefault()
  }, { signal })
})
onBeforeUnmount(() => {
  listeners.abort()
  heldSelection.value = null
})

// The focus in the field ends the selection on the page, which keeps drawing it;
// a picture a menu opened the field on is drawn as selected too.
watch([focused, selection, pictureField], ([now, held, picture]) => {
  if (picture) {
    const range = document.createRange()
    range.selectNode(picture.image)
    heldSelection.value = range
  } else
    heldSelection.value = now && held ? held.range : null
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

const shown = computed(() => current.value !== null && !dragging.value && !phone.value)

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
watch([focusWanted, box, menuOpen, () => isPositioned.value], async ([wanted, field, menu, drawn]) => {
  if (!wanted || !field || menu || !drawn)
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
  'onClose': closeAll,
}))

function onFocusin(event: FocusEvent): void {
  focused.value ||= (event.target as HTMLElement).tagName === 'TEXTAREA'
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="shown && current"
      ref="floating"
      data-selection-field-layer
      :style="{ ...placement, zIndex: z(), ...(menuOpen ? { visibility: 'hidden' } : {}) }"
      @focusin="onFocusin"
    >
      <SelectionField :key="opening" ref="box" v-bind="fieldBindings" />
    </div>
  </Teleport>
</template>
