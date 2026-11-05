<script setup lang="ts">
import { Maximize, Minimize, Pause, Play, Volume1, Volume2, VolumeX } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useIconPx } from '../ui/icon-metrics'
import { clock } from './clock'
import { t } from './locales/text'
import { dropSource } from './preview'

/**
 * A video on its stage, the space it is shown in, with a frosted capsule of
 * controls that floats over the stage's bottom, always there, and takes no
 * room of its own: Play or Pause, the time played, the playhead, the length,
 * the speed, Mute and Full Screen. The picture is drawn in `picture`, its box
 * on the stage; a picture that comes down to the capsule lies under it.
 *
 * With a pointer that hovers, the volume stands above Mute while the pointer
 * is on either or one has the keyboard, as a system player's does, so the
 * capsule keeps its layout; a phone has Mute alone, since an iPhone's page
 * cannot set a video's volume. The volume is the browser's for every video:
 * the next one starts at the last that was set above silence.
 *
 * A click on the picture plays or pauses; with the player or one of
 * its controls focused, Space does but on a button, which it presses, and
 * Left and Right go back or ahead 5 seconds, so a viewer that steps between
 * media with them leaves them to the player (`media-viewer.ts`).
 *
 * Full Screen shows the player, capsule and all, on the whole screen, the
 * picture as large as the screen lets it; an iPhone, which shows no element
 * full screen, plays the video in its own player instead.
 */
const props = defineProps<{
  src: string
  name: string
  autoplay?: boolean
  /** The picture's box on the stage; the whole stage, fitted, while unknown. */
  picture?: { width: string; height: string }
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
const volume = ref(storedVolume())
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
/** What the volume control shows: none while muted. */
const heard = computed(() => (muted.value ? 0 : volume.value))

/** Where the browser keeps the volume every video starts at. */
const VOLUME_KEY = 'demi-video-volume'

function storedVolume(): number {
  if (typeof localStorage === 'undefined')
    return 1
  const value = Number(localStorage.getItem(VOLUME_KEY) ?? Number.NaN)
  return value > 0 && value <= 1 ? value : 1
}

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

/** Unmuting a video whose volume was taken to silence brings back the volume kept. */
function toggleMute(): void {
  const video = media.value
  if (!video)
    return
  if (video.muted && video.volume === 0)
    video.volume = storedVolume()
  video.muted = !video.muted
}

/** Silence is Mute, and any volume above it unmutes, as a system player's slider does. */
function setVolume(event: Event): void {
  const video = media.value
  if (!video)
    return
  const value = Number((event.target as HTMLInputElement).value)
  video.volume = value
  video.muted = value === 0
}

function volumeChanged(): void {
  const video = media.value
  if (!video)
    return
  muted.value = video.muted || video.volume === 0
  volume.value = video.volume
  if (video.volume > 0 && typeof localStorage !== 'undefined')
    localStorage.setItem(VOLUME_KEY, String(video.volume))
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
  // The volume's own arrows move it, and leave the playhead where it is.
  if (event.key.startsWith('Arrow') && event.target instanceof Element && event.target.matches('[data-video-volume]'))
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

onMounted(() => {
  document.addEventListener('fullscreenchange', fullScreenChanged)
  if (media.value)
    media.value.volume = volume.value
})
onBeforeUnmount(() => {
  document.removeEventListener('fullscreenchange', fullScreenChanged)
  if (media.value)
    dropSource(media.value)
})
</script>

<template>
  <div
    ref="root"
    class="video-player relative flex items-center justify-center outline-none"
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
      class="block shrink-0 bg-black object-contain"
      :class="{ 'size-full': !props.picture || fullScreen }"
      :style="fullScreen ? undefined : props.picture"
      @click="toggle"
      @loadedmetadata="loadedMetadata"
      @durationchange="measured"
      @progress="progressed"
      @timeupdate="current = media?.currentTime ?? 0; progressed()"
      @play="playing = true"
      @pause="playing = false"
      @ended="ended"
      @volumechange="volumeChanged"
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
        <Pause v-if="playing" :size="icons.in32" fill="currentColor" stroke-width="0" aria-hidden="true" />
        <Play v-else :size="icons.in32" fill="currentColor" stroke-width="0" aria-hidden="true" />
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
      <div class="video-volume">
        <button type="button" class="video-control" :aria-label="muted ? t('player.unmute') : t('player.mute')" @click="toggleMute">
          <VolumeX v-if="muted" :size="icons.in32" aria-hidden="true" />
          <Volume1 v-else-if="volume < 0.5" :size="icons.in32" aria-hidden="true" />
          <Volume2 v-else :size="icons.in32" aria-hidden="true" />
        </button>
        <div class="video-volume-stand">
          <div class="video-volume-panel">
            <span class="video-volume-track">
              <span class="video-volume-level" :style="{ height: `${heard * 100}%` }" />
              <span class="video-volume-knob" :style="{ bottom: `${heard * 100}%` }" />
              <input
                type="range"
                min="0"
                max="1"
                step="0.05"
                :value="heard"
                :aria-label="t('player.volume')"
                :aria-valuetext="`${Math.round(heard * 100)}%`"
                class="video-volume-input"
                data-video-volume
                @input="setVolume"
              >
            </span>
          </div>
        </div>
      </div>
      <button
        type="button"
        class="video-control"
        :aria-label="fullScreen ? t('player.exitFullScreen') : t('player.enterFullScreen')"
        @click="toggleFullScreen"
      >
        <Minimize v-if="fullScreen" :size="icons.in32" aria-hidden="true" />
        <Maximize v-else :size="icons.in32" aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<style scoped>
/* The capsule's size follows the stage's width. */
.video-player {
  container-type: inline-size;
}

/* Full screen, the stage is black around the picture, as a system player's is. */
.video-player:fullscreen {
  background: #000;
}

/*
 * The capsule: the floating surface at about two thirds over a blur of what
 * lies behind it, the same on a phone, at most 560 px wide and 32 px narrower
 * than a smaller stage, where the playhead gives up the width.
 */
.video-controls {
  position: absolute;
  bottom: 12px;
  left: 50%;
  display: flex;
  width: min(560px, calc(100% - 32px));
  height: 44px;
  align-items: center;
  gap: 8px;
  padding: 0 10px 0 6px;
  border-radius: 9999px;
  color: var(--color-fg);
  transform: translateX(-50%);
  user-select: none;
}

/*
 * The capsule's surface, and the volume's panel, which shares its look. The
 * capsule draws its blur on a layer of its own: an element with a backdrop
 * filter blurs only what lies inside it for its descendants, so the volume's
 * panel, above the capsule, would blur nothing of the picture.
 */
.video-controls::before,
.video-volume-panel {
  background: color-mix(in srgb, var(--color-surface-float) 62%, transparent);
  box-shadow: 0 0 0 0.5px var(--color-line), var(--shadow-float);
  backdrop-filter: blur(20px) saturate(180%);
  -webkit-backdrop-filter: blur(20px) saturate(180%);
}

.video-controls::before {
  position: absolute;
  inset: 0;
  z-index: -1;
  border-radius: inherit;
  content: '';
}

.video-control {
  display: flex;
  min-width: 32px;
  height: 32px;
  flex: none;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  cursor: default;
}

.video-control:focus-visible {
  outline: 2px solid var(--color-accent-fill);
  outline-offset: 2px;
}

/* Inside the stage, which fills its space edge to edge. */
.video-player:focus-visible {
  outline: 2px solid var(--color-accent-fill);
  outline-offset: -2px;
}

.video-volume {
  position: relative;
  display: flex;
  flex: none;
}

/*
 * The volume stands over Mute, 8 px above the capsule, and its stand reaches
 * down to Mute so the pointer keeps it on its way up. It shows only where a
 * pointer hovers: a finger has Mute alone. The keyboard's focus keeps it, a
 * click's does not, so it closes as the pointer leaves after a press of Mute.
 * Closed, it is unseen but stays in the page, so Tab from Mute reaches it: one
 * removed would lose the focus it is taking.
 */
.video-volume-stand {
  position: absolute;
  bottom: 100%;
  left: 50%;
  display: none;
  padding-bottom: 14px;
  transform: translateX(-50%);
}

@media (hover: hover) {
  .video-volume-stand {
    display: block;
    opacity: 0;
    pointer-events: none;
  }

  .video-volume:hover .video-volume-stand,
  .video-volume:has(:focus-visible) .video-volume-stand {
    opacity: 1;
    pointer-events: auto;
  }
}

.video-volume-panel {
  display: flex;
  width: 32px;
  height: 112px;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
}

.video-volume-track {
  position: relative;
  width: 4px;
  height: 80px;
  border-radius: 9999px;
  background: color-mix(in srgb, var(--color-fg) 14%, transparent);
}

.video-volume-level {
  position: absolute;
  inset: auto 0 0;
  border-radius: inherit;
  background: currentColor;
}

/* White in either theme, as the playhead's knob. */
.video-volume-knob {
  position: absolute;
  left: 50%;
  width: 12px;
  height: 12px;
  margin: 0 0 -6px -6px;
  border-radius: 9999px;
  background: #fff;
  box-shadow: 0 0 0 0.5px rgb(0 0 0 / 0.2), 0 1px 3px rgb(0 0 0 / 0.3);
}

/* Unseen over the track, wider than it so a press beside it takes, bottom to top. */
.video-volume-input {
  position: absolute;
  inset: 0 -10px;
  width: auto;
  height: 100%;
  margin: 0;
  appearance: none;
  background: transparent;
  cursor: default;
  opacity: 0;
  writing-mode: vertical-lr;
  direction: rtl;
}

.video-volume-input::-webkit-slider-thumb {
  width: 1rem;
  height: 0;
  appearance: none;
}

.video-volume-input::-moz-range-thumb {
  width: 1rem;
  height: 0;
  border: 0;
}

.video-volume:has(.video-volume-input:focus-visible) .video-volume-panel {
  outline: 2px solid var(--color-accent-fill);
  outline-offset: 2px;
}

.video-rate {
  min-width: 36px;
  font-size: 12px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}

.video-time {
  flex: none;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  text-align: center;
  color: var(--color-fg-muted);
}

.video-track {
  position: relative;
  height: 4px;
  min-width: 0;
  flex: 1;
  border-radius: 9999px;
  background: color-mix(in srgb, var(--color-fg) 14%, transparent);
}

.video-track-loaded,
.video-track-played {
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: inherit;
}

/* What is loaded but not played, between the track and the played part, as a system player draws it. */
.video-track-loaded {
  background: color-mix(in srgb, var(--color-fg) 32%, transparent);
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
  width: 12px;
  height: 12px;
  margin: -6px 0 0 -6px;
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

/*
 * On a stage narrower than 360 px, such as a 320 px phone or the media
 * viewer's window at its narrowest, the capsule takes more of the width and
 * its controls draw closer, so the playhead keeps a length beside an hour's
 * times.
 */
@container (width < 360px) {
  .video-controls {
    width: calc(100% - 16px);
    gap: 2px;
    padding: 0 6px 0 4px;
  }

  .video-control {
    min-width: 28px;
  }

  .video-rate {
    min-width: 30px;
  }
}

/* A finger takes the playhead anywhere up and down the capsule. */
@media (max-width: 767px) {
  .video-seek {
    inset: -20px 0;
  }
}
</style>
