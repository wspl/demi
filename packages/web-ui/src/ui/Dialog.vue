<script setup lang="ts">
import { computed, inject, provide, ref, watch } from 'vue'
import { X } from '@lucide/vue'
import IconButton from './IconButton.vue'
import { t as shared } from '../locales/text'
import ScrollArea from './ScrollArea.vue'
import { useEscapeClose } from '../composables/useEscapeClose'
import { provideRowTarget } from './operations'
import { createOverlayFamily, overlayFamilyKey, overlayOwnerKey } from '../overlay/overlayFamily'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayInlineKey, useOverlayTarget } from '../overlay/overlayContainer'
import { dialogNestingKey } from '../overlay/dialogNesting'
import { usePinnedOpen } from '../overlay/pinnedOpen'
import { DEFAULT_ACTION, useDialogFocus } from '../overlay/dialogFocus'
import { provideLayerElevation } from '../overlay/layerElevation'
import { LAYER_Z, provideStacking } from '../overlay/stacking'
import { useOverlay } from '../composables/useOverlay'
import type { HeadlineText } from './ui-text'

/**
 * Size: md is the default compact panel, wide fits a settings row beside a
 * long input, lg is an editor, xl is the settings shell, full is a viewer
 * that fills the window.
 * Narrow: where the scrim is narrower than the app's medium breakpoint (the
 * width at which the app's side panes become overlays), an xl or full panel
 * fills the scrim edge to edge with square corners and no margin of scrim,
 * keeping its content inside the device's safe area. Smaller sizes stay
 * floating panels. The scrim, not the viewport, is measured, so a catalog
 * host of phone width shows the same.
 * A flow that the whole screen should hold on a phone, such as a wizard, sets
 * `fillNarrow`: its panel then fills the scrim there whatever its size.
 * Inline (a catalog host provides `overlayInlineKey`): the panel renders in flow at its
 * own size, with no scrim and no centering.
 * Focus: a dialog over the page takes the focus when it opens, keeps Tab inside
 * while it is the top layer, and gives the focus back when it closes
 * (`useDialogFocus`). A dialog pinned open in a catalog host (open as the host first renders it)
 * is no layer, takes no focus and hears no Escape; once closed and opened again there,
 * it behaves as the product's: the top layer, with the focus, and Escape closes it.
 * Nesting: a dialog opened from inside another stacks on it; the one beneath stays,
 * Escape and the scrim close only the top, and closing the one beneath takes the
 * stack with it. A menu or dropdown open in a dialog is above it: Escape closes it
 * first and the dialog stays, the next Escape closes the dialog. An Escape a field handled first (it called preventDefault, as a
 * field reverting its edit does) closes nothing.
 * Anchor: centered by default; `top` hangs the panel from a line near the top, for a
 * panel whose height follows its content, such as the search window's list.
 * Scrolling: content scrolls as a whole by default. Set `scrollContent` to false
 * when content owns its scroll region: use a `flex min-h-0 flex-col` root,
 * a non-shrinking header and a shrinking ScrollArea for the body.
 * Actions: the `footer` slot holds a dialog's buttons, laid out as a macOS
 * sheet's at the trailing edge, the default button last and rightmost
 * whatever order the caller writes them in, as NSAlert places it: Cancel
 * before Save, and Remove before Cancel where Cancel is the default;
 * `footer-leading` holds what stands apart at the leading
 * edge, a link or a Delete beside Save. The footer stays in place while the
 * content scrolls, and the content above it ends without its own bottom
 * padding. Return presses the default button, the footer's primary one,
 * from anywhere in the panel, except where the focus is on a control Return
 * acts on itself: a button, a link, a multi-line field, or a field's open
 * completion; a dialog without a field opens with the focus on it. An
 * action that destroys is never the default, as Apple's guidelines and
 * NSAlert have it: in a confirmation of one, Cancel is the primary button,
 * so neither Return nor the opening focus destroys, and the action is a
 * danger button, red with no fill, that takes a click. Escape cancels, as
 * the close control does.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  size?: 'md' | 'wide' | 'lg' | 'xl' | 'full'
  label?: HeadlineText
  /** Every dialog closes from its top-right corner; a flow that must finish can hide it. */
  hideClose?: boolean
  /**
   * Content runs under the close control (a picture, a page that scrolls up to the
   * panel's top edge): the control is raised on an opaque fill, so nothing shows through it.
   */
  closeOverContent?: boolean
  /** Where the scrim is narrow, fill it whatever the size, as an xl or full panel does. */
  fillNarrow?: boolean
  /** False when the slot owns a body scroller beneath a fixed header. */
  scrollContent?: boolean
  /** Stack on whatever dialog is open instead of replacing it, for a dialog mounted at the app root but opened from inside another. */
  stack?: boolean
  /**
   * Top: the panel hangs from a fixed line near the top of the window instead of
   * being centered, so its top stays where it is while content that grows or
   * shrinks changes its height, as Spotlight's and Raycast's windows do.
   */
  anchor?: 'center' | 'top'
}>()

const emit = defineEmits<{
  close: []
}>()

const slots = defineSlots<{
  default(): unknown
  /** The buttons, Cancel before the default button. */
  footer?(): unknown
  /** What stands apart at the leading edge: a link, or Delete beside Save. */
  'footer-leading'?(): unknown
}>()

// A dialog confined to a host container and pinned open never blocks the page, so it is not
// exclusive. One the user opens there is a layer of the page as in the product (`usePinnedOpen`).
const { container, to: teleportTarget, ready } = useOverlayTarget()
// Inline: the panel sits in flow at its own size, with no scrim, for a catalog specimen.
const inline = inject(overlayInlineKey, false)
const family = createOverlayFamily()
// Child menus share ownership, but the dialog surface is outside their click boundary.
provide(overlayFamilyKey, family)
const nested = inject(dialogNestingKey, false)
provide(dialogNestingKey, true)
// A dialog's controls show their own work, even in a row of their target.
provideRowTarget(() => undefined)
// What opens in the dialog floats above it and its cards.
provideLayerElevation()

// A shell or a viewer is a page of its own: on a narrow screen it takes the whole window.
const fillsWhenNarrow = computed(() => props.fillNarrow || props.size === 'xl' || props.size === 'full')

const pinned = usePinnedOpen(() => props.isOpen, container)
const id = useOverlay(
  props.overlayStore,
  () => (pinned.value ? false : props.isOpen),
  () => {
    if (props.isOpen)
      emit('close')
  },
  nested || props.stack ? 'stacked' : 'exclusive',
  'dialog',
)

// What opens inside, such as a dropdown, is a layer on this dialog: Escape closes it first.
provide(overlayOwnerKey, id)

/**
 * How high the dialog stands: a dialog opened later stands above one opened
 * before it, whichever was mounted first, as a confirmation opened over the
 * dialog that asked for it must. A leaving dialog keeps its height while it fades.
 */
const depth = ref(0)
watch(
  () => props.overlayStore.state.entries.findIndex((entry) => entry.id === id),
  (index) => {
    if (index >= 0) {
      depth.value = index
    }
  },
  { immediate: true },
)
// A menu opened in the dialog stands above it, however high the dialog stands.
provideStacking(() => LAYER_Z + depth.value)

const panel = ref<HTMLElement>()
// A dialog pinned open in a catalog host is a specimen beside others, not the page's: it leaves the focus alone.
useDialogFocus({
  panel,
  isOpen: () => props.isOpen && !pinned.value,
  isTop: () => props.overlayStore.isTop(id),
})

/** A control Return acts on itself, so it never reaches the default button. */
const OWN_RETURN = 'button, a[href], select, textarea, [contenteditable]:not([contenteditable=false]), [role=button], [role=link]'

const footer = ref<HTMLElement>()
// Return presses the default button wherever the focus is in the panel, as a sheet's does.
// An input method's Enter never gets here: every text field keeps it (`keepComposition`).
function pressDefault(event: KeyboardEvent): void {
  if (event.key !== 'Enter' || event.defaultPrevented || event.shiftKey || event.altKey || event.ctrlKey || event.metaKey)
    return
  if (!pinned.value && !props.overlayStore.isTop(id))
    return
  if (event.target instanceof Element && event.target.closest(OWN_RETURN) !== null)
    return
  const action = footer.value?.querySelector<HTMLElement>(DEFAULT_ACTION)
  if (!action)
    return
  event.preventDefault()
  action.click()
}

// An Escape a field inside used, to revert its edit or clear its filter, closes nothing.
useEscapeClose(() => props.isOpen && !pinned.value && props.overlayStore.isTop(id), () => emit('close'))
</script>

<template>
  <Teleport v-if="ready" :to="teleportTarget" :disabled="inline">
    <!-- One transition for scrim and panel: a nested one never gets to leave, since the
         outer v-if unmounts the subtree. The panel's scale rides on the same stage classes. -->
    <Transition name="dialog" appear>
      <div
        v-if="isOpen"
        :class="inline
          ? 'dialog-scrim relative grid'
          : ['dialog-scrim dialog-host fixed inset-0 z-50 grid bg-black/40', anchor === 'top' ? 'items-start justify-items-center pt-[12vh]' : 'place-items-center']"
        :style="inline || depth === 0 ? undefined : { zIndex: LAYER_Z + depth }"
        @click.self="!inline && emit('close')"
      >
        <!-- The panel is a region (`ui/region.ts`): a menu opened in it grows toward its inside. -->
        <div
          ref="panel"
          data-region
          class="dialog-panel overlay-dialog relative flex flex-col overflow-hidden rounded-xl bg-surface-dialog shadow-2xl outline-none"
          :class="[
            inline ? 'w-full' : 'max-h-[calc(100%-2rem)] w-[calc(100%-2rem)]',
            !inline && fillsWhenNarrow && 'dialog-fill',
            size === 'full' ? 'h-[calc(100%-2rem)]' : size === 'xl' ? 'max-w-5xl' : size === 'lg' ? 'max-w-3xl' : size === 'wide' ? 'max-w-xl' : 'max-w-md',
          ]"
          role="dialog"
          tabindex="-1"
          :aria-modal="container ? undefined : 'true'"
          :aria-label="label"
          @keydown="pressDefault"
        >
          <div v-if="!hideClose" class="dialog-close absolute right-3 top-3 z-10">
            <IconButton
              :icon="X"
              :variant="closeOverContent ? 'solid' : 'ghost'"
              :aria-label="shared('common.close')"
              @click="emit('close')"
            />
          </div>
          <!-- Content with its own scrolling body shrinks inside; anything else scrolls as a whole. -->
          <ScrollArea
            v-if="scrollContent !== false"
            class="min-h-0"
            viewport-class="flex flex-col"
          >
            <slot />
          </ScrollArea>
          <slot v-else />
          <footer
            v-if="slots.footer || slots['footer-leading']"
            ref="footer"
            class="flex shrink-0 items-center gap-2 px-5 pb-5 pt-4 phone:flex-wrap"
          >
            <!-- On a phone the buttons are a finger's size and never cut short: where the
                 leading ones and the rest do not fit one row, the rest take the next, at its end. -->
            <div v-if="slots['footer-leading']" class="flex min-w-0 items-center gap-2 phone:min-w-max">
              <slot name="footer-leading" />
            </div>
            <!-- The default button stands last, whichever it is (`Button` marks it). -->
            <div class="ml-auto flex shrink-0 items-center gap-2 [&>:has([data-default-action])]:order-last">
              <slot name="footer" />
            </div>
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.dialog-enter-active,
.dialog-leave-active {
  transition: opacity 150ms ease-out;
}

.dialog-enter-active .dialog-panel,
.dialog-leave-active .dialog-panel {
  transition: opacity 150ms ease-out, transform 150ms ease-out;
}

.dialog-enter-from,
.dialog-leave-to {
  opacity: 0;
}

.dialog-enter-from .dialog-panel,
.dialog-leave-to .dialog-panel {
  opacity: 0;
  transform: scale(0.95);
}

/* A closed dialog is gone for the pointer at once, as a macOS sheet is: its
   fading scrim would otherwise take the next press meant for the page. */
.dialog-leave-active,
.dialog-leave-active * {
  pointer-events: none;
}

/* The scrim is the container a panel fills; an inline panel has no scrim to fill. */
.dialog-host {
  container-type: inline-size;
}

/* 48rem is Tailwind's md breakpoint, the app's narrow width (SidebarLayout). */
@container (width < 48rem) {
  .dialog-fill {
    width: 100%;
    max-width: none;
    height: 100%;
    max-height: none;
    border-radius: 0;
    padding: var(--safe-top) var(--safe-right) var(--safe-bottom) var(--safe-left);
  }

  /* The close button keeps its corner inset inside the safe area. */
  .dialog-fill .dialog-close {
    top: calc(0.75rem + var(--safe-top));
    right: calc(0.75rem + var(--safe-right));
  }

  /* On a phone it stands centred on the 44 px bar at the panel's top, such
     as Settings' back row, as far from the edge as a bar's buttons. */
  @media (max-width: 767px) {
    .dialog-fill .dialog-close {
      top: calc((2.75rem - var(--spacing-hit)) / 2 + var(--safe-top));
      right: calc(0.375rem + var(--safe-right));
    }
  }

  /* A panel that fills the window fades without scaling, so no scrim shows at its edges. */
  .dialog-enter-from .dialog-fill,
  .dialog-leave-to .dialog-fill {
    transform: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .dialog-enter-active,
  .dialog-leave-active,
  .dialog-enter-active .dialog-panel,
  .dialog-leave-active .dialog-panel {
    transition: none;
  }
}
</style>
