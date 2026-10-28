<script setup lang="ts">
import { DOCS, REPOSITORY } from '@/links'
import DemiLogo from './DemiLogo.vue'
import GitHubMark from './GitHubMark.vue'
</script>

<template>
  <header class="nav">
    <a class="brand" href="/"><DemiLogo otter /></a>
    <nav class="links" aria-label="Site">
      <a href="#features">Features</a>
      <a :href="DOCS">Docs</a>
    </nav>
    <a class="repo" :href="REPOSITORY" aria-label="Demi on GitHub">
      <GitHubMark class="mark" />
    </a>
  </header>
</template>

<style scoped>
.nav {
  position: relative;
  z-index: 2;
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  height: 68px;
  padding: 0 var(--hero-inset);
}

.brand {
  justify-self: start;
  color: var(--ink);
}

.links {
  display: flex;
  gap: clamp(22px, 2.8vw, 40px);
  font-size: 14.5px;
  font-weight: 500;
}

.links a,
.repo {
  color: var(--ink);
  text-decoration: none;
}

.links a:hover {
  opacity: 0.7;
}

.repo {
  justify-self: end;
}

.mark {
  width: 26px;
  height: 26px;
}

@media (max-width: 720px) {
  .nav {
    grid-template-columns: 1fr auto;
  }

  .links {
    display: none;
  }
}
</style>
