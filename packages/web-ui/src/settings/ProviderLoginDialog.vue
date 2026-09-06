<script setup lang="ts">
import { ref } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, Copy, ExternalLink, TriangleAlert } from '@lucide/vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/**
 * A subscription sign-in, as the vendor's own flow: a device code confirmed in a
 * browser, a code the user copies back, or a token the vendor's CLI hands out after
 * its own login. Owns nothing; the host drives the phase.
 */
export type ProviderLoginPhase =
  | { kind: 'starting' }
  | { kind: 'device'; url: string; code: string; expiresIn: string }
  | { kind: 'code-input'; url: string }
  | {
      kind: 'token'
      /** The command that prints the token, run in the user's own terminal. */
      command: string
      /** Where to get the CLI when it is missing. */
      install: { label: string; url: string }
      /** What a token starts with, so a paste can be checked before it is sent. */
      prefix: string
    }
  | { kind: 'done'; account: string }
  | { kind: 'failed'; message: string }

defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  vendorName: string
  phase: ProviderLoginPhase
}>()

const emit = defineEmits<{
  close: []
  submitCode: [code: string]
  submitToken: [token: string]
  /** The host opens the vendor's page the way it opens any external link. */
  open: [url: string]
  retry: []
}>()

const pasted = ref('')
const token = ref('')
const { copy, copied } = useClipboard({ copiedDuring: 1500 })
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="`Sign in to ${vendorName}`" @close="emit('close')">
    <div class="flex flex-col gap-5 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Sign in to {{ vendorName }}</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          <template v-if="phase.kind === 'device'">Enter this code on the vendor's page. Demi keeps waiting here.</template>
          <template v-else-if="phase.kind === 'code-input'">Approve in the browser, then paste the code it shows you.</template>
          <template v-else-if="phase.kind === 'token'">Sign in with the vendor's own tool, then paste the token it prints.</template>
          <template v-else-if="phase.kind === 'done'">Signed in. This account is now active for {{ vendorName }}.</template>
          <template v-else-if="phase.kind === 'failed'">The sign-in did not complete.</template>
          <template v-else>Contacting {{ vendorName }}…</template>
        </p>
      </header>

      <div v-if="phase.kind === 'starting'" class="flex items-center gap-2 py-4 text-chrome text-fg-muted">
        <IndeterminateSpinner :size="ICON_PX.in24" />
        Requesting a sign-in code
      </div>

      <div v-else-if="phase.kind === 'device'" class="flex flex-col gap-4">
        <div class="flex items-center justify-between rounded-xl border border-line bg-surface px-4 py-3">
          <span class="select-all font-mono text-[22px] tracking-[0.25em] text-fg-emphasis">{{ phase.code }}</span>
          <IconButton :icon="copied ? Check : Copy" variant="ghost" :aria-label="copied ? 'Copied' : 'Copy code'" @click="copy(phase.code)" />
        </div>
        <div class="flex flex-wrap items-center gap-3">
          <Button variant="primary" @click="emit('open', phase.url)">
            Open in browser
            <ExternalLink :size="ICON_PX.in24" />
          </Button>
          <span class="flex items-center gap-1.5 text-[12px] text-fg-subtle">
            <IndeterminateSpinner :size="ICON_PX.in20" />
            Waiting · code expires in {{ phase.expiresIn }}
          </span>
        </div>
      </div>

      <div v-else-if="phase.kind === 'code-input'" class="flex flex-col gap-3">
        <Button class="self-start" @click="emit('open', phase.url)">
          Open in browser
          <ExternalLink :size="ICON_PX.in24" />
        </Button>
        <div class="flex items-center gap-2">
          <TextInput v-model="pasted" placeholder="Paste the code here" class="flex-1" />
          <Button variant="primary" :disabled="!pasted.trim()" @click="emit('submitCode', pasted.trim())">Continue</Button>
        </div>
      </div>

      <!-- Token: three numbered steps; the command copies, the last step is the paste. -->
      <ol v-else-if="phase.kind === 'token'" class="flex flex-col gap-3">
        <li class="flex items-start gap-3">
          <Tag class="mt-0.5 shrink-0 tabular-nums">1</Tag>
          <div class="flex min-w-0 flex-1 flex-col gap-1.5">
            <span class="text-chrome text-fg">Install {{ vendorName }} if you have not.</span>
            <Button class="self-start" @click="emit('open', phase.install.url)">
              {{ phase.install.label }}
              <ExternalLink :size="ICON_PX.in24" />
            </Button>
          </div>
        </li>
        <li class="flex items-start gap-3">
          <Tag class="mt-0.5 shrink-0 tabular-nums">2</Tag>
          <div class="flex min-w-0 flex-1 flex-col gap-1.5">
            <span class="text-chrome text-fg">Run this in a terminal and sign in when the browser opens.</span>
            <div class="flex items-center gap-2 rounded-md bg-surface px-3 py-1.5 ring-1 ring-line">
              <code class="min-w-0 flex-1 select-all truncate font-mono text-[12px] text-fg">{{ phase.command }}</code>
              <IconButton size="sm" :icon="copied ? Check : Copy" variant="ghost" :aria-label="copied ? 'Copied' : 'Copy command'" @click="copy(phase.command)" />
            </div>
          </div>
        </li>
        <li class="flex items-start gap-3">
          <Tag class="mt-0.5 shrink-0 tabular-nums">3</Tag>
          <div class="flex min-w-0 flex-1 flex-col gap-1.5">
            <span class="text-chrome text-fg">Paste the token it prints.</span>
            <div class="flex items-center gap-2">
              <TextInput v-model="token" secret :placeholder="`${phase.prefix}…`" class="flex-1" @keydown.enter="token.trim().startsWith(phase.prefix) && emit('submitToken', token.trim())" />
              <Button variant="primary" :disabled="!token.trim().startsWith(phase.prefix)" @click="emit('submitToken', token.trim())">Continue</Button>
            </div>
            <span v-if="token.trim() && !token.trim().startsWith(phase.prefix)" class="text-[12px] text-on-danger">A token starts with {{ phase.prefix }}.</span>
          </div>
        </li>
      </ol>

      <div v-else-if="phase.kind === 'done'" class="flex items-center gap-2 py-2 text-chrome text-fg">
        <Check :size="ICON_PX.in28" class="text-on-success" />
        {{ phase.account }}
      </div>

      <div v-else class="flex items-start gap-2 rounded-xl bg-tint-danger px-3 py-2 text-[12px] leading-5 text-on-danger">
        <TriangleAlert :size="ICON_PX.in24" class="mt-0.5 shrink-0" />
        {{ phase.message }}
      </div>

      <div class="flex justify-end gap-2">
        <Button v-if="phase.kind === 'done'" variant="primary" @click="emit('close')">Done</Button>
        <template v-else>
          <Button variant="ghost" @click="emit('close')">Cancel</Button>
          <Button v-if="phase.kind === 'failed'" @click="emit('retry')">Try again</Button>
        </template>
      </div>
    </div>
  </Dialog>
</template>
