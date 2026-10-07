<script setup lang="ts">
import { CircleStop } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'

/** Where the user stopped the turn: a decision, not a failure (`product.md` § Recovering an unfinished turn). */
</script>

<template>
  <FunctionalBlock>
    <template #icon>
      <CircleStop :size="ICON_PX.in28" />
    </template>
    <span class="min-w-0 truncate">Stopped</span>
  </FunctionalBlock>
</template>
