<script setup lang="ts">
// The last screen: the install command, with the otter resting its paws on
// it, then the footer. A pale scene of otters loading a boat lies in the
// bottom-left corner, under the empty space beside and below the content.
import { BookOpenIcon } from '@lucide/vue'
import CommandBox from '@/components/CommandBox.vue'
import DemiLogo from '@/components/DemiLogo.vue'
import GitHubMark from '@/components/GitHubMark.vue'
import RequirementsNote from '@/components/RequirementsNote.vue'
import { DOCS, LICENSE, REPOSITORY } from '@/links'
import otter from '@/assets/otter-peeking.webp'
import pier1x from '@/assets/pier-1x.webp'
import pier2x from '@/assets/pier-2x.webp'
</script>

<template>
  <div class="last">
  <section class="install">
    <img
      class="scene"
      data-decor
      :src="pier1x"
      :srcset="`${pier1x} 1019w, ${pier2x} 2038w`"
      sizes="min(56vw, 133vh)"
      alt=""
      width="2038"
      height="744"
      loading="lazy"
    />
    <h2 class="section-title">Up and running in minutes.</h2>
    <p class="section-lede">Install Demi on your own server with one command.</p>
    <div class="box">
      <img class="otter" :src="otter" alt="" width="240" height="232" />
      <CommandBox />
    </div>
    <div class="actions">
      <a class="button dark" :href="REPOSITORY"><GitHubMark class="mark" />View on GitHub</a>
      <a class="button light" :href="DOCS"><BookOpenIcon :size="16" />Read the Docs</a>
    </div>
    <RequirementsNote class="needs" distributions />
  </section>

  <footer class="footer">
    <div class="row">
      <DemiLogo class="brand" />
      <a :href="DOCS">Docs</a>
      <a :href="REPOSITORY">GitHub</a>
      <a :href="LICENSE">License</a>
    </div>
  </footer>
  </div>
</template>

<style scoped>
/* The last screen: the install call and the footer fill one viewport. */
.last {
  display: flex;
  flex-direction: column;
  min-height: 100svh;
}

.install {
  position: relative;
  display: flex;
  flex: 1;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: clamp(1.25rem, 4vh, 3rem) clamp(1.25rem, 3.6vw, 4rem);
  text-align: center;
}

/* The scene stays clear of the content, in a short window as in a narrow
   one: its tall left end beside the content, its water low under it. It is
   pale enough to read as part of the paper. */
.scene {
  position: absolute;
  bottom: 0;
  left: 0;
  width: min(56vw, 133vh);
  height: auto;
  opacity: 0.25;
  pointer-events: none;
  user-select: none;
}

.install > :not(.scene) {
  position: relative;
}

.section-lede {
  margin-right: auto;
  margin-left: auto;
}

.box {
  position: relative;
  max-width: 100%;
  margin-top: 6.75rem;
}

.otter {
  position: absolute;
  top: 0;
  left: 50%;
  z-index: 1;
  width: 7.5rem;
  height: auto;
  /* The body's flat bottom edge, 91.5% down the image, sits on the box's
     top edge, so only the paws reach over it. */
  transform: translate(-50%, -91.5%);
  pointer-events: none;
}

.box :deep(.command) {
  text-align: left;
}

.actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 0.75rem;
  margin-top: 1.5rem;
}

.button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  min-width: 11rem;
  padding: 0.6875rem 1.25rem;
  border-radius: 0.625rem;
  font-size: 0.9375rem;
  font-weight: 500;
  text-decoration: none;
}

.dark {
  background: #2a2624;
  color: #fff;
}

.dark:hover {
  background: #3a3532;
}

.light {
  border: 1px solid var(--line-strong);
  background: #f8f4ef;
}

.light:hover {
  background: #f1ebe3;
}

.mark {
  width: 1.125rem;
  height: 1.125rem;
}

.needs {
  margin-top: 1rem;
  font-size: 0.8438rem;
}

.footer {
  margin: 0 clamp(1.25rem, 3.6vw, 4rem);
  padding: 2.25rem 0 2.75rem;
  border-top: 1px solid var(--line);
  text-align: center;
}

.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 0;
}

.row > * {
  padding: 0 1.5rem;
  border-left: 1px solid var(--line);
}

.row > :first-child {
  border-left: 0;
}

.brand {
  color: var(--ink);
}

.row a {
  color: var(--ink-soft);
  font-size: 0.9375rem;
  text-decoration: none;
}

.row a:hover {
  color: var(--ink);
}


/* A narrow window has no room beside the content for the scene. */
@media (max-width: 900px) {
  .scene {
    display: none;
  }
}

@media (max-width: 720px) {
  .box {
    margin-top: 5.5rem;
  }

  .otter {
    width: 6rem;
  }

  .button {
    min-width: 0;
    width: 100%;
  }

  .row > * {
    padding: 0 1rem;
  }

  .brand {
    flex-basis: 100%;
    justify-content: center;
    margin-bottom: 0.625rem;
  }

  .row > :nth-child(2) {
    border-left: 0;
  }

  .row a {
    font-size: 1.0625rem;
  }
}
</style>
