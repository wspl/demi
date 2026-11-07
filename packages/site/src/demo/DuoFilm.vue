<script setup lang="ts">
// iPhone Duo's view in the Simulators window, as the demo shows it: a film of the
// device in 3D, rendered ahead from its own body, since the live view is a
// WebGL canvas a recording cannot keep. It plays once from when it shows.
defineProps<{ src: string }>()
</script>

<template>
  <video class="film" :src="src" autoplay muted playsinline preload="auto" />
</template>

<style scoped>
.film {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
  /* The panel's own sunken view, which the film was rendered on. */
  background: #f8f4f1;
}
</style>
