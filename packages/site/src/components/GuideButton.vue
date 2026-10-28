<script setup lang="ts">
// The hero's one dark button, Get Started: to the installation guide, the
// next step after the command above it.
import { ArrowRightIcon } from '@lucide/vue'
import { DOCS } from '@/links'
</script>

<template>
  <a class="guide" :href="DOCS">
    <span>Get Started</span>
    <ArrowRightIcon :size="17" class="arrow" />
  </a>
</template>

<style scoped>
.guide {
  display: inline-flex;
  align-items: center;
  gap: 0.625rem;
  padding: 0.75rem 1.125rem 0.75rem 1.375rem;
  border-radius: 62.4375rem;
  background: var(--button);
  color: #fbf7f0;
  font-weight: 500;
  text-decoration: none;
  transition: background-color 0.15s;
}

.guide:hover {
  background: #3d2d23;
}

.arrow {
  transition: transform 0.15s;
}

.guide:hover .arrow {
  transform: translateX(2px);
}
</style>
