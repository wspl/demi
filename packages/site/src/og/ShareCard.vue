<script setup lang="ts">
// The share image (`SHARE_IMAGE` in `seo.ts`, 1200 by 630): the logo and the
// first screen's heading over the first screen's demo, on the page's shore.
import DemiLogo from '@/components/DemiLogo.vue'
import DemoStage from '@/components/DemoStage.vue'
import shore from '@/assets/hero-shore-2x.webp'
import { SITE } from '@/seo'
</script>

<template>
  <div class="card">
    <img class="shore" :src="shore" alt="" />
    <div class="head">
      <DemiLogo class="logo" otter />
      <h1>{{ SITE.headline }}</h1>
    </div>
    <DemoStage class="stage" demo="hero" label="" />
  </div>
</template>

<style scoped>
.card {
  position: relative;
  width: 1200px;
  height: 630px;
  overflow: hidden;
  background: var(--cream);
}

.shore {
  position: absolute;
  bottom: 0;
  left: 0;
  width: 100%;
}

.head {
  position: relative;
  padding-top: 52px;
  text-align: center;
}

.logo {
  zoom: 1.25;
}

h1 {
  margin-top: 20px;
  font-size: 60px;
  font-weight: 650;
  letter-spacing: -0.035em;
  line-height: 1;
}

.stage {
  position: absolute;
  top: 244px;
  left: 150px;
  width: 900px;
}
</style>
