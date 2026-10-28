<script setup lang="ts">
// What the server needs, with "Linux server" linked to the distributions it
// can run; `distributions` also names them on a second line.
import { DISTRIBUTIONS, DISTRIBUTIONS_URL, REQUIREMENTS } from '@/links'

defineProps<{ distributions?: boolean }>()

const [before, after] = REQUIREMENTS.split('Linux server')
</script>

<template>
  <p class="requirements">
    {{ before }}<a :href="DISTRIBUTIONS_URL">Linux server</a>{{ after }}
    <template v-if="distributions"><br />{{ DISTRIBUTIONS }}</template>
  </p>
</template>

<style scoped>
.requirements {
  margin: 0;
  color: var(--ink-faint);
  font-size: 14px;
  line-height: 1.6;
}

.requirements a {
  color: var(--ink-soft);
  text-underline-offset: 3px;
}
</style>
