<script setup lang="ts">
import { useRouter } from 'vue-router'
import Button from '@demicodes/web-ui/ui/Button.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { useResources } from '../prototype/resources'
const resources = useResources()
const router = useRouter()
function enter() {
  if (!resources.username.trim()) return
  resources.signedIn = true
  void router.push('/chat/welcome')
}
</script>

<template>
  <main class="grid h-full place-items-center bg-surface-base">
    <form class="w-80 space-y-5 p-4" @submit.prevent="enter">
      <h1 class="text-[20px] font-medium text-fg-emphasis">Welcome to Demi</h1>
      <p class="text-chrome text-fg-subtle">A place to think and build.</p>
      <label class="flex flex-col gap-2 text-chrome text-fg-muted">
        Your name
        <TextInput
          v-model="resources.username"
          required
          maxlength="50"
          autocomplete="nickname"
          focused
        />
      </label>
      <Button variant="primary" :disabled="!resources.username.trim()" @click="enter">
        Enter prototype
      </Button>
      <p class="text-[11px] text-fg-faint">Local demo · No account or password required.</p>
    </form>
  </main>
</template>
