<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import HandoffPage, { type HandoffPagePhase } from '@demicodes/web-ui/auth/HandoffPage.vue'
import { spendHandoffCode } from './handoff'

/**
 * The page a phone's camera opens from a handoff code (`backend.md`
 * § Authentication and ownership): the code is after `#`, which a browser
 * never sends to a server. The page takes it off the address, so the
 * history keeps no code, spends it, and loads the page it opens, signed in:
 * a load, since the phone may have been signed out, or in to another
 * account, whose state the page held. A phone signed in to the account
 * keeps its session.
 */
const router = useRouter()
const phase = ref<HandoffPagePhase>({ kind: 'opening' })

onMounted(async () => {
  const code = window.location.hash.slice(1)
  history.replaceState(history.state, '', window.location.pathname)
  if (!code) {
    phase.value = { kind: 'expired' }
    return
  }
  const spent = await spendHandoffCode(code)
  switch (spent.kind) {
    case 'handedOff':
      window.location.replace(spent.to)
      break
    case 'expired':
      phase.value = { kind: 'expired' }
      break
    case 'failed':
      phase.value = spent
      break
  }
})
</script>

<template>
  <HandoffPage :phase="phase" @sign-in="router.replace('/login')" />
</template>
