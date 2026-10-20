<script setup lang="ts">
import { t } from './locales/text'
import { computed, inject, onBeforeUnmount, provide, ref, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { ChevronDown } from '@lucide/vue'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import IconButton from '../ui/IconButton.vue'
import RequestOutputMeter from './RequestOutputMeter.vue'
import { sessionCoverKey } from './session-cover'
import { dockCardsKey } from './dock-cards'

withDefaults(defineProps<{
  showScrollToBottom?: boolean
  /**
   * Reads the server's reading of the request the root agent runs, which the
   * row shows at its right end. A getter, not a value: the response's own
   * count comes in one batch with the turn's end, and a prop would carry
   * only the last of them.
   */
  output?: () => RequestOutputReading | null
  /** The conversation the reading belongs to. */
  scope?: string
}>(), {
  showScrollToBottom: false,
  output: () => null,
  scope: '',
})

const emit = defineEmits<{
  scrollToBottom: []
}>()

// What waits for a decision stands over the dock's top and takes no room in it,
// so a panel the dock opened keeps its size and place under it. The surface
// keeps the transcript clear of it.
const coverRef = ref<HTMLElement>()
const { height: coverHeight } = useElementSize(coverRef, { width: 0, height: 0 }, { box: 'border-box' })
const surfaceCover = inject(sessionCoverKey, null)
watch(coverHeight, (height) => {
  if (surfaceCover) {
    surfaceCover.value = height
  }
}, { immediate: true })
onBeforeUnmount(() => {
  if (surfaceCover) {
    surfaceCover.value = 0
  }
})

// The composer's cards stand here, above the chips, so the chips sit directly
// on the input.
const cardsRef = ref<HTMLElement>()
provide(dockCardsKey, cardsRef)

// The scroll control belongs to the row of chips, right after the chips, or at
// the row's left end with none. While the row shows, it stands level with it;
// without, it floats where that row would be, above what waits over the
// composer.
const rowRef = ref<HTMLElement>()
const chipsRef = ref<HTMLElement>()
const composerRef = ref<HTMLElement>()
const { height: rowHeight } = useElementSize(rowRef, { width: 0, height: 0 }, { box: 'border-box' })
const { width: chipsWidth } = useElementSize(chipsRef, { width: 0, height: 0 }, { box: 'border-box' })
const { height: composerHeight } = useElementSize(composerRef, { width: 0, height: 0 }, { box: 'border-box' })
/** The chips row's step over the composer (its `pb-2`). */
const STEP_PX = 8
/** The gap between the row's parts (its `gap-1.5`). */
const GAP_PX = 6
const scrollBottom = computed(() =>
  rowHeight.value > 0
    ? `${composerHeight.value + STEP_PX}px`
    : `calc(100% + ${coverHeight.value + STEP_PX}px)`,
)
const scrollLeft = computed(() => (chipsWidth.value > 0 ? `${chipsWidth.value + GAP_PX}px` : '0px'))
</script>

<template>
  <!--
    What stands over the composer stacks at one 8px step: the cards, the chips,
    the composer.

    - The chips sit directly on the composer, and nothing comes between them.
    - A card stands above the chips.
    - A part that is not there takes no room, so the next one moves up by the
      same step.
    - A press in the dock, on the composer or its empty space, closes the newest
      window as one on the transcript does. Only the Agents and Running chips
      keep it.
  -->
  <div class="session-dock relative">
    <!--
      The scroll control is the chips row's control after its chips, so it
      takes no room in the dock. It shows only away from the scroll bottom,
      where it covers nothing the user reads last.
    -->
    <span
      class="pointer-events-none absolute z-20 inline-flex size-hit"
      :style="{ bottom: scrollBottom, left: scrollLeft }"
    >
      <Transition
        appear
        :duration="200"
        enter-active-class="transition-opacity duration-200 ease-out"
        leave-active-class="transition-opacity duration-200 ease-out"
        enter-from-class="opacity-0"
        leave-to-class="opacity-0"
      >
        <IconButton
          v-if="showScrollToBottom"
          class="pointer-events-auto"
          :icon="ChevronDown"
          variant="solid"
          circle
          :aria-label="t('dock.scrollToBottom')"
          @click="emit('scrollToBottom')"
          @transitionend.stop
        />
      </Transition>
    </span>
    <!--
      What waits for the user's decision sits over the chips, below the
      transcript, and over the lower part of an open panel.
    -->
    <div v-if="$slots.above" ref="coverRef" class="absolute inset-x-0 bottom-full pb-2">
      <slot name="above" />
    </div>
    <!--
      The composer's cards, such as the offline primary Host's. Each brings its
      own step under it.
    -->
    <div ref="cardsRef" class="flex flex-col" />
    <!--
      Chips read from the left, then the scroll control's room while it shows,
      then the reading at the right end, its end over the send button's. A chip
      or the reading renders nothing while it has nothing to say, so the row
      shows only while one of its parts holds an element.

      The chips keep their room: on a narrow row the reading gives way, its
      speed first.
    -->
    <div ref="rowRef" class="hidden items-center gap-1.5 pb-2 has-[>*>*]:flex">
      <div ref="chipsRef" class="flex shrink-0 items-center gap-1.5">
        <slot name="chips" />
      </div>
      <span v-if="showScrollToBottom" class="w-hit shrink-0" />
      <div class="flex min-w-0 flex-1 justify-end pe-[var(--composer-inset)]">
        <RequestOutputMeter :output="output" :scope="scope" />
      </div>
    </div>
    <div ref="composerRef" class="relative z-10">
      <slot />
    </div>
  </div>
</template>
