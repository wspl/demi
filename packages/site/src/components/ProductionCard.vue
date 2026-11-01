<script setup lang="ts">
// A short film's production as an editor's timeline at the browser's
// corner, beside a demo whose scene says how many stages are done and
// whether the next runs: a ruler of the film's 15 seconds over a video
// track and two audio tracks. The storyboard's shots land on the video
// track as stills, the voiceover and the music fill their tracks, the stills
// turn into clips, and the cut sweeps the playhead across, as each stage
// runs. The track a stage works on is lit.
import { ClapperboardIcon, FilmIcon, Mic2Icon, MusicIcon } from '@lucide/vue'
import shot1 from '@/demo/assets/shot-1.webp'
import shot2 from '@/demo/assets/shot-2.webp'
import shot3 from '@/demo/assets/shot-3.webp'
import type { StageCard } from '@/demo/message'
import { useKept } from './stageCard'

type Production = Extract<StageCard, { kind: 'production' }>

const props = defineProps<{ card: Production | null }>()

const last = useKept(() => props.card, { kind: 'production', done: 0, running: false })

const SHOTS = [shot1, shot2, shot3]
/** The stages, in the order the scene runs them. */
const STORYBOARD = 0
const AUDIO = 1
const CLIPS = 2
const CUT = 3
/** Whether stage `stage` is done, or runs now. */
const done = (stage: number) => last.value.done > stage
const running = (stage: number) => last.value.done === stage && last.value.running

/** The ruler's marks, a second apart, every fifth one named. */
const MARKS = Array.from({ length: 16 }, (_, second) => second)
/** The waveforms' bars, the same every play: the voice in phrases, the music steady. */
const VOICE = Array.from({ length: 40 }, (_, i) => (i % 13 > 9 ? 0.08 : 0.25 + 0.75 * Math.abs(Math.sin(i * 1.9) * Math.cos(i * 0.7))))
const MUSIC = Array.from({ length: 40 }, (_, i) => 0.45 + 0.35 * Math.abs(Math.sin(i * 0.8)))
</script>

<template>
  <div class="editor stage-card" :class="{ shown: card !== null }" aria-hidden="true">
    <div class="bar">
      <ClapperboardIcon :size="13" :stroke-width="1.8" />
      <span>teaser</span>
      <span class="time">{{ done(CUT) ? '00:15' : '00:00' }} / 00:15</span>
    </div>
    <div class="grid">
      <span class="corner" />
      <div class="ruler">
        <span v-for="second in MARKS" :key="second" class="mark" :class="{ major: second % 5 === 0 }" :style="{ left: `${(second / 15) * 100}%` }">
          <b v-if="second % 5 === 0 && second < 15">0:{{ String(second).padStart(2, '0') }}</b>
        </span>
      </div>

      <span class="head" :class="{ lit: running(STORYBOARD) || running(CLIPS) }"><FilmIcon :size="11" />V1</span>
      <div class="lane" :class="{ lit: running(STORYBOARD) || running(CLIPS) }">
        <span
          v-for="(shot, i) in SHOTS"
          :key="i"
          class="clip video"
          :class="{ placed: done(STORYBOARD) || running(STORYBOARD), moving: done(CLIPS), making: running(CLIPS) }"
          :style="{ '--i': i, left: `${(i / 3) * 100}%` }"
        >
          <b>shot-{{ i + 1 }}</b>
          <span class="frames"><img v-for="n in 3" :key="n" :src="shot" alt="" /></span>
        </span>
      </div>

      <span class="head" :class="{ lit: running(AUDIO) }"><Mic2Icon :size="11" />A1</span>
      <div class="lane" :class="{ lit: running(AUDIO) }">
        <span class="clip audio voice" :class="{ placed: done(AUDIO) || running(AUDIO) }">
          <b>voice</b>
          <span class="wave"><i v-for="(bar, i) in VOICE" :key="i" :style="{ height: `${bar * 100}%` }" /></span>
        </span>
      </div>

      <span class="head" :class="{ lit: running(AUDIO) }"><MusicIcon :size="11" />A2</span>
      <div class="lane" :class="{ lit: running(AUDIO) }">
        <span class="clip audio music" :class="{ placed: done(AUDIO) || running(AUDIO) }">
          <b>music</b>
          <span class="wave"><i v-for="(bar, i) in MUSIC" :key="i" :style="{ height: `${bar * 100}%` }" /></span>
        </span>
      </div>

      <span class="playhead" :class="{ sweeping: running(CUT), end: done(CUT) }" />
    </div>
  </div>
</template>

<style scoped>
/* The card is drawn as an editor's timeline, dark as editors are, so it reads as one at a glance. */
.editor {
  --track: 2.1rem;
  --heads: 2.6rem;

  width: 40%;
  min-width: 18rem;
  overflow: hidden;
  border-radius: 0.75rem;
  background: #24201e;
  box-shadow: 0 2px 0.25rem rgb(60 40 20 / 8%), 0 1.25rem 3rem rgb(40 25 15 / 26%);
  color: #d9d1c8;
  font: 500 0.7rem/1.2 var(--sans);
}

.bar {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid #3a3431;
  color: #f2ece6;
}

.time {
  margin-left: auto;
  color: #a39890;
  font: 400 0.65rem/1 var(--mono);
  font-variant-numeric: tabular-nums;
}

.grid {
  position: relative;
  display: grid;
  grid-template-columns: var(--heads) 1fr;
  row-gap: 3px;
  padding: 0 0.6rem 0.65rem 0;
}

.corner {
  border-right: 1px solid #3a3431;
}

.ruler {
  position: relative;
  height: 1.3rem;
  margin-bottom: 2px;
  border-bottom: 1px solid #3a3431;
}

.mark {
  position: absolute;
  bottom: 0;
  width: 1px;
  height: 0.25rem;
  background: #5a524d;
}

.mark.major {
  height: 0.5rem;
  background: #8a7f78;
}

.mark b {
  position: absolute;
  bottom: 0.55rem;
  left: 0.2rem;
  color: #8a7f78;
  font: 400 0.6rem/1 var(--mono);
  white-space: nowrap;
}

/* Each track's head on the left: lit while a stage works on its track. */
.head {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  height: var(--track);
  padding-left: 0.6rem;
  border-right: 1px solid #3a3431;
  color: #8a7f78;
  font: 500 0.62rem/1 var(--mono);
  transition: color 0.3s ease;
}

.head.lit {
  color: #f0a95b;
}

.lane {
  position: relative;
  height: var(--track);
  margin-left: 4px;
  border-radius: 0.25rem;
  background: #2d2826;
  transition: background 0.3s ease;
}

.lane.lit {
  background: #352c26;
}

/* A clip on a track: it drops into place when its stage makes it. */
.clip {
  position: absolute;
  top: 2px;
  bottom: 2px;
  overflow: hidden;
  border-radius: 0.2rem;
  opacity: 0;
  transform: translateY(-0.4rem);
  transition: opacity 0.35s ease, transform 0.45s cubic-bezier(0.16, 1, 0.3, 1);
}

.clip.placed {
  opacity: 1;
  transform: none;
}

/* A clip's name on a strip of its own along its top, as editors put it, over none of its content. */
.clip {
  display: flex;
  flex-direction: column;
}

.clip b {
  flex: none;
  padding: 1px 0.25rem 2px;
  color: rgb(255 255 255 / 88%);
  font: 600 0.55rem/1 var(--mono);
}

/* The storyboard's stills, then the clips made of them: a strip of frames that fills each third. */
.clip.video {
  width: calc(100% / 3 - 2px);
  border: 1px solid #5d7fa3;
  background: #3b5878;
  transition-delay: calc(var(--i) * 0.2s);
}

.frames {
  display: flex;
  flex: 1;
  min-height: 0;
}

.frames img {
  flex: 1;
  min-width: 0;
  object-fit: cover;
  opacity: 0.9;
}

/* A still is one frame; a clip shows its frames side by side, as a filmstrip. */
.clip.video:not(.moving) .frames img:not(:first-child) {
  display: none;
}

.clip.video:not(.moving) .frames img {
  flex: none;
  width: 2rem;
}

.clip.video.making {
  animation: render 0.8s ease-in-out infinite alternate;
  animation-delay: calc(var(--i) * 0.2s);
}

@keyframes render {
  to {
    filter: brightness(1.35);
  }
}

.clip.audio {
  left: 0;
}

.clip.voice {
  width: 72%;
  border: 1px solid #6f9a6a;
  background: #3f5a3c;
}

.clip.music {
  width: 100%;
  border: 1px solid #a3825d;
  background: #5c4630;
  transition-delay: 0.2s;
}

.wave {
  display: flex;
  flex: 1;
  align-items: center;
  gap: 1px;
  min-height: 0;
  padding: 0 0.25rem 2px;
}

.wave i {
  flex: 1;
  max-height: 100%;
  border-radius: 1px;
  background: rgb(255 255 255 / 55%);
}

/* The playhead runs the film once the cut is made. */
.playhead {
  position: absolute;
  top: 0;
  bottom: 0.4rem;
  left: var(--heads);
  width: 2px;
  margin-left: 4px;
  background: #f0a95b;
  opacity: 0.35;
  pointer-events: none;
}

.playhead::before {
  content: "";
  position: absolute;
  top: 0;
  left: -4px;
  border: 5px solid transparent;
  border-top: 6px solid #f0a95b;
}

.playhead.sweeping,
.playhead.end {
  opacity: 1;
}

.playhead.sweeping {
  animation: sweep 1.3s linear both;
}

.playhead.end {
  left: calc(100% - 0.6rem - 2px);
}

@keyframes sweep {
  to {
    left: calc(100% - 0.6rem - 2px);
  }
}

@media (prefers-reduced-motion: reduce) {
  .editor,
  .editor.shown,
  .clip {
    transition: none;
  }

  .clip.video.making,
  .playhead.sweeping {
    animation: none;
  }
}
</style>
