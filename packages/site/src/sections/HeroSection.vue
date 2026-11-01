<script setup lang="ts">
// The first screen, as tall as the window: what Demi is and how to install
// it, beside the product itself, a desktop browser and a phone playing the
// same conversation. A pale shore along the bottom, the otter afloat in it,
// is decoration only: it stays low and washed out, under the product.
import CommandBox from '@/components/CommandBox.vue'
import GuideButton from '@/components/GuideButton.vue'
import RequirementsNote from '@/components/RequirementsNote.vue'
import DemoStage from '@/components/DemoStage.vue'
import SiteNav from '@/components/SiteNav.vue'
import { SITE } from '@/seo'
import shore1x from '@/assets/hero-shore-1x.webp'
import shore2x from '@/assets/hero-shore-2x.webp'
import shore3x from '@/assets/hero-shore-3x.webp'

/** The shore at three widths, so a sharp or wide screen gets enough pixels. */
const SRCSET = `${shore1x} 2172w, ${shore2x} 4344w, ${shore3x} 6516w`
</script>

<template>
  <section class="hero">
    <img
      class="shore"
      :src="shore1x"
      :srcset="SRCSET"
      sizes="max(100vw, 1100px)"
      alt=""
      width="2172"
      height="406"
    />

    <SiteNav />

    <div class="body">
      <div class="copy">
        <h1>{{ SITE.headline }}</h1>
        <p class="lede">
          Demi is a web-based AI agent you host on your own server. Open it in any browser. It works
          across all your computers and keeps going after you close the tab.
        </p>
        <CommandBox class="command" />
        <div class="actions">
          <GuideButton />
          <RequirementsNote class="requirements" />
        </div>
      </div>
      <DemoStage class="stage" demo="hero" lifts label="From a browser, Demi fixes a bug on a Mac and tests it on a Windows PC; from a phone, it ships the release to a VPS and copies the VPS's data to a home lab." />
    </div>
  </section>
</template>

<style scoped>
.hero {
  --hero-inset: clamp(20px, 4.2vw, 80px);
  /* The shore's height: the image is 5.35 times as wide as it is tall. */
  --shore: calc(max(100vw, 1100px) / 5.35);

  position: relative;
  display: flex;
  flex-direction: column;
  min-height: max(620px, 100svh);
  overflow: hidden;
}

/* Along the bottom, the full width; its horizon is about halfway up. */
.shore {
  position: absolute;
  bottom: 0;
  left: 0;
  width: max(100vw, 1100px);
  max-width: none;
  height: auto;
  pointer-events: none;
}

.body {
  position: relative;
  display: grid;
  flex: 1;
  grid-template-columns: minmax(0, 0.85fr) minmax(0, 1.15fr);
  gap: clamp(28px, 4vw, 80px);
  align-items: center;
  padding: 0 var(--hero-inset) calc(var(--shore) * 0.18);
}

/* A very wide window keeps the product at a size that reads; the copy stays beside it. */
.stage {
  width: 88%;
  max-width: 920px;
  justify-self: center;
}

h1 {
  font-size: clamp(38px, min(4.4vw, 7.4vh), 80px);
  font-weight: 650;
  letter-spacing: -0.035em;
  line-height: 0.98;
}

.copy {
  max-width: 640px;
}

.lede {
  max-width: 34em;
  margin-top: clamp(14px, 2.4vh, 22px);
  color: var(--ink-soft);
  font-size: clamp(15px, min(1.25vw, 2.1vh), 20px);
  line-height: 1.5;
}

.copy .command {
  max-width: 600px;
  margin-top: clamp(16px, 2.8vh, 28px);
  font-size: clamp(12px, min(0.95vw, 1.7vh), 16px);
}

.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px 18px;
  margin-top: clamp(14px, 2.4vh, 22px);
}

/* Beside the button when the sentence fits on one line, under it otherwise. */
.requirements {
  white-space: nowrap;
}

/* A narrow window stands the product under the copy. */
/* A narrower window gives the copy half the width, so the install command
   stays whole on one line. */
@media (max-width: 1200px) {
  .body {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }
}

@media (max-width: 900px) {
  .body {
    grid-template-columns: minmax(0, 1fr);
    align-content: start;
    padding-top: clamp(16px, 4vh, 40px);
  }
}
</style>
