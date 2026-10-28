<script setup lang="ts">
import Interlude from './components/Interlude.vue'
import FeatureTour from './sections/FeatureTour.vue'
import HeroSection from './sections/HeroSection.vue'
import InstallSection from './sections/InstallSection.vue'
import ProvidersSection from './sections/ProvidersSection.vue'
import YoursSection from './sections/YoursSection.vue'
import key1x from './assets/key-1x.webp'
import key2x from './assets/key-2x.webp'
import plane1x from './assets/plane-1x.webp'
import plane2x from './assets/plane-2x.webp'
import plug1x from './assets/plug-1x.webp'
import plug2x from './assets/plug-2x.webp'
</script>

<template>
  <HeroSection />
  <main>
    <FeatureTour />
    <!-- Work sent on from machine to machine. -->
    <Interlude :src1x="plane1x" :src2x="plane2x" :width="2054" :height="304" side="right" />
    <YoursSection />
    <!-- The key to your own place. -->
    <Interlude :src1x="key1x" :src2x="key2x" :width="2071" :height="322" side="left" />
    <ProvidersSection />
    <!-- Plug in what you already have. -->
    <Interlude :src1x="plug1x" :src2x="plug2x" :width="2055" :height="397" side="right" />
    <InstallSection />
  </main>
</template>
