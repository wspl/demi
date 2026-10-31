<script setup lang="ts">
import { Pause, Play } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, useId, watch } from 'vue'
import { ICON_PX } from '../ui/icon-metrics'
import { t } from './locales/text'
import { dropSource } from './preview'
import { waveformOf } from './waveform'

/**
 * An audio file played where it shows, as a voice message plays in
 * Messages: a play button, its name, its length, which counts the time
 * played once it plays, and its waveform, which fills with the accent as it
 * plays and seeks where it is pressed or dragged. The waveform is a row of
 * hairlines, flat until the file is read. Leaving the page stops it.
 */
const props = defineProps<{
  src: string
  name: string
}>()

const media = ref<HTMLAudioElement | null>(null)
const playing = ref(false)
const current = ref(0)
/** Its length in seconds once the file's header is read; null before, or for one that does not say. */
const length = ref<number | null>(null)

/** How many hairlines the waveform has, and how far apart they stand in its drawing's units. */
const BARS = 80
const STRIDE = 3
const WIDTH = BARS * STRIDE
/** The file's loudness along its length, flat until it is read. */
const peaks = ref<number[] | null>(null)
/** The reading this player waits for, given up when its file changes or the player goes. */
let reading: AbortController | undefined
watch(() => props.src, (src) => {
  reading?.abort()
  peaks.value = null
  if (!src)
    return
  const mine = new AbortController()
  reading = mine
  void waveformOf(src, BARS, mine.signal).then((read) => {
    if (!mine.signal.aborted)
      peaks.value = read
  })
}, { immediate: true })
/** Each hairline's height, as a share of the waveform's: a short one where nothing is read yet or nothing sounds. */
const heights = computed(() => Array.from({ length: BARS }, (_, i) => Math.max(0.12, peaks.value?.[i] ?? 0)))
/** How much of it has played, 0 to 1. */
const played = computed(() => (length.value ? Math.min(1, current.value / length.value) : 0))
const clipId = useId()

/** `0:04`, `12:30`: whole seconds in minutes and seconds, as a player shows them. */
function clock(whole: number): string {
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`
}

/** The time it shows: the time played once it started, its length before. */
const time = computed(() => {
  // The time played counts whole seconds; the length is the nearest, as
  // `demi audio` says it.
  if (playing.value || current.value > 0)
    return clock(Math.floor(current.value))
  return length.value === null ? '–:––' : clock(Math.round(length.value))
})

function measured(): void {
  const duration = media.value?.duration ?? Number.NaN
  length.value = Number.isFinite(duration) ? duration : null
}

function toggle(): void {
  const audio = media.value
  if (!audio)
    return
  if (audio.paused) {
    // A play the browser refuses, such as one with no sound device, leaves it paused, which the button shows.
    void audio.play().catch(() => {})
  } else {
    audio.pause()
  }
}

function seek(event: Event): void {
  const audio = media.value
  if (audio && event.target instanceof HTMLInputElement) {
    audio.currentTime = Number(event.target.value)
    current.value = audio.currentTime
  }
}

function ended(): void {
  playing.value = false
  current.value = 0
}

onBeforeUnmount(() => {
  reading?.abort()
  if (media.value)
    dropSource(media.value)
})
</script>

<template>
  <!--
    The round button is as tall as the name and the waveform beside it, so
    the padding is the same above, below and left of it.
  -->
  <span
    class="audio-player inline-flex w-[min(100%,320px)] select-none items-center gap-2.5 rounded-2xl border border-line bg-surface-float p-1.5 pr-3 align-middle"
    data-audio-player
  >
    <audio
      ref="media"
      :src="props.src"
      preload="metadata"
      @loadedmetadata="measured"
      @durationchange="measured"
      @timeupdate="current = media?.currentTime ?? 0"
      @play="playing = true"
      @pause="playing = false"
      @ended="ended"
    />
    <button
      type="button"
      class="flex size-9 shrink-0 cursor-default items-center justify-center rounded-full bg-accent-fill text-white"
      :aria-label="playing ? t('audioPlayer.pause', { name: props.name }) : t('audioPlayer.play', { name: props.name })"
      @click="toggle"
    >
      <Pause v-if="playing" :size="ICON_PX.in28" fill="currentColor" aria-hidden="true" />
      <Play v-else :size="ICON_PX.in28" fill="currentColor" aria-hidden="true" class="translate-x-px" />
    </button>
    <span class="flex min-w-0 flex-1 flex-col gap-1">
      <span class="flex h-4 min-w-0 items-baseline gap-2">
        <span class="min-w-0 truncate text-[13px] leading-4 text-fg">{{ props.name }}</span>
        <span class="ml-auto shrink-0 text-[12px] leading-4 tabular-nums text-fg-subtle" data-audio-time>{{ time }}</span>
      </span>
      <!-- The waveform, with the seek bar over it, unseen, which takes the pointer and the keys. -->
      <span class="audio-wave relative block h-4">
        <svg class="block size-full overflow-visible" :viewBox="`0 0 ${WIDTH} 16`" preserveAspectRatio="none" aria-hidden="true">
          <defs>
            <clipPath :id="clipId">
              <rect x="0" y="-1" :width="WIDTH * played" height="18" />
            </clipPath>
          </defs>
          <g class="text-fg/20" fill="currentColor">
            <rect v-for="(height, i) in heights" :key="i" :x="i * STRIDE + 0.9" :y="8 - height * 8" width="1.2" :height="height * 16" rx="0.6" />
          </g>
          <g class="text-accent-fill" fill="currentColor" :clip-path="`url(#${clipId})`">
            <rect v-for="(height, i) in heights" :key="i" :x="i * STRIDE + 0.9" :y="8 - height * 8" width="1.2" :height="height * 16" rx="0.6" />
          </g>
        </svg>
        <input
          type="range"
          min="0"
          :max="length ?? 0"
          step="any"
          :value="current"
          :disabled="length === null"
          :aria-label="t('audioPlayer.seek', { name: props.name })"
          class="audio-seek absolute inset-0 size-full cursor-default opacity-0"
          @input="seek"
        >
      </span>
    </span>
  </span>
</template>

<style scoped>
/*
 * The seek bar is unseen; its focus shows as a ring around the waveform. Its
 * thumb takes no width, so its track spans the waveform end to end and a
 * press seeks to the time drawn under it.
 */
.audio-seek {
  appearance: none;
  background: transparent;
}

.audio-seek::-webkit-slider-thumb {
  width: 0;
  height: 1rem;
  appearance: none;
}

.audio-seek::-moz-range-thumb {
  width: 0;
  height: 1rem;
  border: 0;
}

.audio-wave:has(.audio-seek:focus-visible) {
  border-radius: 0.25rem;
  outline: 2px solid var(--color-accent-fill);
  outline-offset: 2px;
}
</style>
