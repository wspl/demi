<script setup lang="ts">
// Demi's brand as the brand assets draw it: the wordmark, after the otter
// when `otter` is set. The two parts come from the lockup and the wordmark
// files, and are sized and spaced here, since the lockup's own spacing is too
// tight at the size of a page's header.
import lockup from '../../../../docs/assets/brand/lockup-light.svg?raw'
import wordmark from '../../../../docs/assets/brand/wordmark-light.svg?url&no-inline'

defineProps<{ otter?: boolean }>()

// The lockup's first nested drawing is the otter.
const otterSvg = /<svg x="0"[\s\S]*?<\/svg>/.exec(lockup)?.[0] ?? ''
</script>

<template>
  <span class="logo" role="img" aria-label="Demi">
    <span v-if="otter" class="otter" v-html="otterSvg" />
    <span class="word" :style="{ '--wordmark': `url('${wordmark}')` }" />
  </span>
</template>

<style scoped>
.logo {
  display: inline-flex;
  align-items: center;
  gap: 10px;
}

.otter {
  display: block;
  width: 32px;
  height: 32px;
}

.otter :deep(svg) {
  display: block;
  width: 100%;
  height: 100%;
}

/* The wordmark's box is 802.51 by 288.01. */
.word {
  display: block;
  width: calc(20px * 802.51 / 288.01);
  height: 20px;
  background: currentColor;
  mask: var(--wordmark) center / contain no-repeat;
  -webkit-mask: var(--wordmark) center / contain no-repeat;
}
</style>
