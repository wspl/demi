<script setup lang="ts">
import { Pause, Play } from '@lucide/vue'
import { computed, onBeforeUnmount, ref } from 'vue'
import { ICON_PX } from '../ui/icon-metrics'
import { t } from './locales/text'
import { dropSource } from './preview'

/**
 * An audio file played where it shows, as a voice message plays in
 * Messages (`file-previews.md` § Files named in messages): its name, its
 * length, which counts the time played once it plays, a play button and a
 * seek bar. Leaving the page stops it.
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
  if (media.value)
    dropSource(media.value)
})
</script>

<template>
  <span
    class="audio-player inline-flex w-[min(100%,320px)] select-none items-center gap-2.5 rounded-2xl border border-line bg-surface-float py-1.5 pl-1.5 pr-3 align-middle"
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
      class="flex size-8 shrink-0 cursor-default items-center justify-center rounded-full bg-accent-fill text-white"
      :aria-label="playing ? t('audioPlayer.pause', { name: props.name }) : t('audioPlayer.play', { name: props.name })"
      @click="toggle"
    >
      <Pause v-if="playing" :size="ICON_PX.in28" fill="currentColor" aria-hidden="true" />
      <Play v-else :size="ICON_PX.in28" fill="currentColor" aria-hidden="true" class="translate-x-px" />
    </button>
    <span class="flex min-w-0 flex-1 flex-col gap-1">
      <span class="flex min-w-0 items-baseline gap-2">
        <span class="min-w-0 truncate text-[13px] leading-4 text-fg">{{ props.name }}</span>
        <span class="ml-auto shrink-0 text-[12px] leading-4 tabular-nums text-fg-subtle" data-audio-time>{{ time }}</span>
      </span>
      <input
        type="range"
        min="0"
        :max="length ?? 0"
        step="any"
        :value="current"
        :disabled="length === null"
        :aria-label="t('audioPlayer.seek', { name: props.name })"
        class="audio-seek h-1 w-full cursor-default accent-[var(--color-accent-fill)]"
        @input="seek"
      >
    </span>
  </span>
</template>
