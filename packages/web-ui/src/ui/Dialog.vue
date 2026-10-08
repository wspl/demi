<script setup lang="ts">
import { computed, inject, provide, ref, watch } from 'vue'
import { X } from '@lucide/vue'
import IconButton from './IconButton.vue'
import ScrollArea from './ScrollArea.vue'
import { onKeyStroke } from '@vueuse/core'
import { createOverlayFamily, overlayFamilyKey } from '../overlay/overlayFamily'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayInlineKey, useOverlayTarget } from '../overlay/overlayContainer'
import { dialogNestingKey } from '../overlay/dialogNesting'
import { DEFAULT_ACTION, useDialogFocus } from '../overlay/dialogFocus'
import { provideLayerElevation } from '../overlay/layerElevation'
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
 * Inline (a catalog host provides `overlayInlineKey`): the panel renders in flow at its
 * own size, with no scrim and no centering.
 * Focus: a dialog over the page takes the focus when it opens, keeps Tab inside
 * while it is the top layer, and gives the focus back when it closes
 * (`useDialogFocus`). One in a catalog host takes no focus.
 * Nesting: a dialog opened from inside another stacks on it; the one beneath stays,
 * Escape and the scrim close only the top, and closing the one beneath takes the
 * stack with it. An Escape a field handled first (it called preventDefault, as a
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

// A dialog confined to a host container never blocks the page, so it is not exclusive.
const { container, to: teleportTarget, ready } = useOverlayTarget()
// Inline: the panel sits in flow at its own size, with no scrim, for a catalog specimen.
const inline = inject(overlayInlineKey, false)
const family = createOverlayFamily()
// Child menus share ownership, but the dialog surface is outside their click boundary.
provide(overlayFamilyKey, family)
const nested = inject(dialogNestingKey, false)
provide(dialogNestingKey, true)
// What opens in the dialog floats above it and its cards.
provideLayerElevation()

// A shell or a viewer is a page of its own: on a narrow screen it takes the whole window.
const fillsWhenNarrow = computed(() => props.size === 'xl' || props.size === 'full')

const id = useOverlay(
  props.overlayStore,
  () => (container ? false : props.isOpen),
  () => {
    if (props.isOpen)
      emit('close')
  },
  nested || props.stack ? 'stacked' : 'exclusive',
  'dialog',
)

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

const panel = ref<HTMLElement>()
// A dialog in a catalog host is a specimen beside others, not the page's: it leaves the focus alone.
if (!container) {
  useDialogFocus({
    panel,
    isOpen: () => props.isOpen,
    isTop: () => props.overlayStore.isTop(id),
  })
}

/** A control Return acts on itself, so it never reaches the default button. */
const OWN_RETURN = 'button, a[href], select, textarea, [contenteditable]:not([contenteditable=false]), [role=button], [role=link]'

const footer = ref<HTMLElement>()
// Return presses the default button wherever the focus is in the panel, as a sheet's does.
function pressDefault(event: KeyboardEvent): void {
  if (event.key !== 'Enter' || event.defaultPrevented || event.isComposing || event.shiftKey || event.altKey || event.ctrlKey || event.metaKey)
    return
  if (!container && !props.overlayStore.isTop(id))
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
onKeyStroke('Escape', (event) => {
  if (!props.isOpen || container || !props.overlayStore.isTop(id) || event.defaultPrevented)
    return
  event.preventDefault()
  emit('close')
})
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
        :style="inline || depth === 0 ? undefined : { zIndex: 50 + depth }"
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
              aria-label="Close"
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
            class="flex shrink-0 items-center gap-2 px-5 pb-5 pt-4"
          >
            <div v-if="slots['footer-leading']" class="flex min-w-0 items-center gap-2">
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
    padding: env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom) env(safe-area-inset-left);
  }

  /* The close button keeps its corner inset inside the safe area. */
  .dialog-fill .dialog-close {
    top: calc(0.75rem + env(safe-area-inset-top));
    right: calc(0.75rem + env(safe-area-inset-right));
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
