<script setup lang="ts">
// A card headed by an otter icon, a title and a line under it; its slot holds
// what the card shows.
defineProps<{ icon: string, title: string, text?: string }>()
</script>

<template>
  <article class="card">
    <header>
      <img :src="icon" alt="" />
      <div>
        <h3>{{ title }}</h3>
        <p v-if="text">{{ text }}</p>
      </div>
    </header>
    <div class="content">
      <slot />
    </div>
  </article>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  padding: 1rem;
  border: 1px solid var(--line);
  border-radius: 1.125rem;
  background: #f6f1ea;
}

header {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.75rem;
  align-items: start;
}

/* As tall as the title and its line; a scene of several otters is wider. */
img {
  height: 3.5rem;
  width: auto;
}

h3 {
  font-size: 1.25rem;
  font-weight: 650;
  letter-spacing: -0.03em;
  line-height: 1.1;
}

p {
  margin-top: 0.25rem;
  color: var(--ink-soft);
  font-size: 0.8438rem;
  line-height: 1.45;
}

.content {
  display: flex;
  flex: 1;
  flex-direction: column;
  margin-top: 0.875rem;
}
</style>
