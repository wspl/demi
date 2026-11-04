<script setup lang="ts">
import { Maximize, Minimize, Pause, Play, Volume2, VolumeX } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useIconPx } from '../ui/icon-metrics'
import { clock } from './clock'
import { t } from './locales/text'
import { dropSource } from './preview'

/**
 * A video under a frosted capsule of controls that floats over the bottom of
 * its picture, always there, and takes no room of its own: Play or Pause,
 * the time played, the playhead, the length, the speed, Mute and Full
 * Screen. A click on the picture plays or pauses; with the player or one of
 * its controls focused, Space does but on a button, which it presses, and
 * Left and Right go back or ahead 5 seconds, so a viewer that steps between
 * media with them leaves them to the player (`media-viewer.ts`).
 *
 * Full Screen shows the player, capsule and all, on the whole screen; an
 * iPhone, which shows no element full screen, plays the video in its own
 * player instead.
 */
const props = defineProps<{
  src: string
  name: string
  autoplay?: boolean
}>()
const emit = defineEmits<{ loadedmetadata: [video: HTMLVideoElement]; error: [] }>()

const root = ref<HTMLElement | null>(null)
const media = ref<HTMLVideoElement | null>(null)
defineExpose({ media })
const icons = useIconPx()

const playing = ref(false)
const current = ref(0)
/** Its length in seconds once the file's header is read; null before, or for one that does not say. */
const length = ref<number | null>(null)
/** How far from the start the file is loaded, in seconds. */
const loaded = ref(0)
const muted = ref(false)
const rate = ref(1)
const fullScreen = ref(false)

/** The speeds the speed control goes round. */
const RATES = [1, 1.5, 2]
/** How far Left and Right move the playhead, in seconds. */
const STEP_SECONDS = 5

const played = computed(() => (length.value ? Math.min(1, current.value / length.value) : 0))
const loadedShare = computed(() => (length.value ? Math.min(1, loaded.value / length.value) : 0))
const lengthText = computed(() => (length.value === null ? '–:––' : clock(Math.round(length.value))))
/** Both times as wide as the length, so the playhead keeps its place as the time played grows. */
const timeWidth = computed(() => `${lengthText.value.length}ch`)

function measured(): void {
  const duration = media.value?.duration ?? Number.NaN
  length.value = Number.isFinite(duration) ? duration : null
}

function progressed(): void {
  const video = media.value
  if (!video)
    return
  const ranges = video.buffered
  loaded.value = ranges.length ? ranges.end(ranges.length - 1) : 0
}

function toggle(): void {
  const video = media.value
  if (!video)
    return
  if (video.paused) {
    // A play the browser refuses leaves it paused, which the button shows.
    void video.play().catch(() => {})
  } else {
    video.pause()
  }
}

function seekTo(seconds: number): void {
  const video = media.value
  if (!video)
    return
  video.currentTime = Math.max(0, Math.min(length.value ?? seconds, seconds))
  current.value = video.currentTime
}

function seek(event: Event): void {
  if (event.target instanceof HTMLInputElement)
    seekTo(Number(event.target.value))
}

function nextRate(): void {
  const video = media.value
  if (video)
    video.playbackRate = RATES[(RATES.indexOf(rate.value) + 1) % RATES.length] ?? 1
}

function toggleMute(): void {
  if (media.value)
    media.value.muted = !media.value.muted
}

/** An iPhone's video element, which plays full screen in the system's player. */
type SystemFullScreen = HTMLVideoElement & { webkitEnterFullscreen?: () => void }

function toggleFullScreen(): void {
  if (document.fullscreenElement) {
    void document.exitFullscreen().catch(() => {})
    return
  }
  if (root.value && document.fullscreenEnabled)
    void root.value.requestFullscreen().catch(() => {})
  else
    (media.value as SystemFullScreen | null)?.webkitEnterFullscreen?.()
}

function fullScreenChanged(): void {
  fullScreen.value = document.fullscreenElement === root.value
}

/** Space plays or pauses but on a button, which it presses; Left and Right move the playhead, the playhead's own keys too. */
function key(event: KeyboardEvent): void {
  if (event.metaKey || event.ctrlKey || event.altKey)
    return
  if (event.key === ' ' && !(event.target instanceof HTMLButtonElement)) {
    event.preventDefault()
    toggle()
  } else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
    event.preventDefault()
    seekTo(current.value + (event.key === 'ArrowLeft' ? -STEP_SECONDS : STEP_SECONDS))
  }
}

function loadedMetadata(): void {
  measured()
  if (media.value)
    emit('loadedmetadata', media.value)
}

function ended(): void {
  playing.value = false
}

onMounted(() => document.addEventListener('fullscreenchange', fullScreenChanged))
onBeforeUnmount(() => {
  document.removeEventListener('fullscreenchange', fullScreenChanged)
  if (media.value)
    dropSource(media.value)
})
</script>

<template>
  <div
    ref="root"
    class="video-player relative bg-black outline-none"
    tabindex="0"
    role="group"
    :aria-label="props.name"
    data-video-player
    @keydown="key"
  >
    <video
      ref="media"
      :src="props.src"
      :autoplay="props.autoplay"
      preload="metadata"
      playsinline
      class="block size-full object-contain"
      @click="toggle"
      @loadedmetadata="loadedMetadata"
      @durationchange="measured"
      @progress="progressed"
      @timeupdate="current = media?.currentTime ?? 0; progressed()"
      @play="playing = true"
      @pause="playing = false"
      @ended="ended"
      @volumechange="muted = media?.muted ?? false"
      @ratechange="rate = media?.playbackRate ?? 1"
      @error="emit('error')"
    />
    <div class="video-controls" data-video-controls>
      <button
        type="button"
        class="video-control"
        :aria-label="playing ? t('player.pause', { name: props.name }) : t('player.play', { name: props.name })"
        @click="toggle"
      >
        <Pause v-if="playing" :size="icons.in20" fill="currentColor" stroke-width="0" aria-hidden="true" />
        <Play v-else :size="icons.in20" fill="currentColor" stroke-width="0" aria-hidden="true" />
      </button>
      <span class="video-time" :style="{ minWidth: timeWidth }">{{ clock(Math.floor(current)) }}</span>
      <span class="video-track">
        <span class="video-track-loaded" :style="{ width: `${loadedShare * 100}%` }" />
        <span class="video-track-played" :style="{ width: `${played * 100}%` }" />
        <span class="video-knob" :style="{ left: `${played * 100}%` }" />
        <input
          type="range"
          min="0"
          :max="length ?? 0"
          step="any"
          :value="current"
          :disabled="length === null"
          :aria-label="t('player.seek', { name: props.name })"
          class="video-seek"
          @input="seek"
        >
      </span>
      <span class="video-time" :style="{ minWidth: timeWidth }">{{ lengthText }}</span>
      <button type="button" class="video-control video-rate" :aria-label="`${t('player.speed')} ${rate}×`" @click="nextRate">
        {{ rate }}×
      </button>
      <button type="button" class="video-control" :aria-label="muted ? t('player.unmute') : t('player.mute')" @click="toggleMute">
        <VolumeX v-if="muted" :size="icons.in20" aria-hidden="true" />
        <Volume2 v-else :size="icons.in20" aria-hidden="true" />
      </button>
      <button
        type="button"
        class="video-control"
        :aria-label="fullScreen ? t('player.exitFullScreen') : t('player.enterFullScreen')"
        @click="toggleFullScreen"
      >
        <Minimize v-if="fullScreen" :size="icons.in20" aria-hidden="true" />
        <Maximize v-else :size="icons.in20" aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<style scoped>
/*
 * The capsule: the floating surface at about two thirds over a blur of the
 * picture behind it, at most 360 px wide and 24 px narrower than a smaller
 * picture, where the playhead gives up the width.
 */
.video-controls {
  position: absolute;
  bottom: 12px;
  left: 50%;
  display: flex;
  width: min(360px, calc(100% - 24px));
  height: 30px;
  align-items: center;
  gap: 6px;
  padding: 0 6px 0 5px;
  border-radius: 9999px;
  color: var(--color-fg);
  background: color-mix(in srgb, var(--color-surface-float) 62%, transparent);
  box-shadow: 0 0 0 0.5px var(--color-line), var(--shadow-float);
  transform: translateX(-50%);
  backdrop-filter: blur(20px) saturate(180%);
  -webkit-backdrop-filter: blur(20px) saturate(180%);
  user-select: none;
}

.video-control {
  display: flex;
  min-width: 20px;
  height: 20px;
  flex: none;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  cursor: default;
}

.video-control:focus-visible,
.video-player:focus-visible {
  outline: 2px solid var(--color-accent-fill);
  outline-offset: 2px;
}

.video-rate {
  min-width: 26px;
  font-size: 10.5px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}

.video-time {
  flex: none;
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  text-align: center;
  color: var(--color-fg-muted);
}

.video-track {
  position: relative;
  height: 3px;
  min-width: 0;
  flex: 1;
  border-radius: 9999px;
  background: color-mix(in srgb, var(--color-fg) 16%, transparent);
}

.video-track-loaded,
.video-track-played {
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: inherit;
}

.video-track-loaded {
  background: color-mix(in srgb, var(--color-fg) 12%, transparent);
}

.video-track-played {
  background: currentColor;
}

/*
 * The knob, white in either theme as a system player's is, shows while the
 * pointer is on the capsule, a finger holds the playhead or the playhead has
 * the keyboard.
 */
.video-knob {
  position: absolute;
  top: 50%;
  width: 10px;
  height: 10px;
  margin: -5px 0 0 -5px;
  border-radius: 9999px;
  background: #fff;
  box-shadow: 0 0 0 0.5px rgb(0 0 0 / 0.2), 0 1px 3px rgb(0 0 0 / 0.3);
  opacity: 0;
}

.video-controls:hover .video-knob,
.video-track:has(.video-seek:active) .video-knob,
.video-track:has(.video-seek:focus-visible) .video-knob {
  opacity: 1;
}

/*
 * The seek bar is unseen over the line, taller than it so a press near it
 * takes; its thumb takes no width, so a press seeks to the time drawn under
 * it.
 */
.video-seek {
  position: absolute;
  inset: -8px 0;
  width: 100%;
  margin: 0;
  appearance: none;
  background: transparent;
  cursor: default;
  opacity: 0;
}

.video-seek::-webkit-slider-thumb {
  width: 0;
  height: 1rem;
  appearance: none;
}

.video-seek::-moz-range-thumb {
  width: 0;
  height: 1rem;
  border: 0;
}

@media (max-width: 767px) {
  .video-controls {
    height: 40px;
    gap: 8px;
    padding: 0 10px 0 8px;
  }

  .video-control {
    min-width: 28px;
    height: 28px;
  }

  /* A finger takes the playhead anywhere up and down the capsule. */
  .video-seek {
    inset: -18px 0;
  }

  .video-rate,
  .video-time {
    font-size: 12px;
  }
}
</style>
