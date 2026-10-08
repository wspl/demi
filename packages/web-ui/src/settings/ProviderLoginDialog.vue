<script setup lang="ts">
import { ref, watch } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, Copy, ExternalLink, Link } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/**
 * A subscription sign-in, as the vendor's own flow: a device code confirmed in a
 * web browser, or the vendor's sign-in page, whose code the user pastes back
 * (`providers.md`). Owns nothing; the host drives the phase.
 */
import type { ProviderLoginPhase } from './types'
export type { ProviderLoginPhase } from './types'

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  vendorName: string
  phase: ProviderLoginPhase
}>()

const emit = defineEmits<{
  close: []
  submitCode: [code: string]
  /** The host opens the vendor's page the way it opens any external link. */
  open: [url: string]
  retry: []
}>()

const code = ref('')
watch(
  () => [props.isOpen, props.phase.kind],
  () => {
    code.value = ''
  },
)

function submitCode(): void {
  if (props.phase.kind === 'code' && !props.phase.submitted && code.value) {
    emit('submitCode', code.value)
  }
}
const { copy, copied } = useClipboard({ copiedDuring: 1500 })
// The link has its own tick: copying it must not mark the code as copied.
const { copy: copyLink, copied: linkCopied } = useClipboard({ copiedDuring: 1500 })
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="`Sign In to ${vendorName}`"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-5 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">
          Sign In to {{ vendorName }}
        </h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          <template v-if="phase.kind === 'device'"
            >Enter this code on the vendor’s page. Demi keeps waiting
            here.</template
          >
          <template v-else-if="phase.kind === 'code'"
            >Sign in on the vendor’s page, then paste the code it
            shows.</template
          >
          <template v-else-if="phase.kind === 'done' && phase.active"
            >Signed in. This account is now active for
            {{ vendorName }}.</template
          >
          <template v-else-if="phase.kind === 'done'"
            >Signed in. {{ vendorName }} keeps using its active account;
            activate this one to switch.</template
          >
          <template v-else-if="phase.kind === 'failed'"
            >The sign-in did not complete.</template
          >
          <template v-else>Contacting {{ vendorName }}…</template>
        </p>
      </header>

      <div
        v-if="phase.kind === 'starting'"
        class="flex items-center gap-2 py-4 text-chrome text-fg-muted"
      >
        <IndeterminateSpinner :size="ICON_PX.in24" />
        Starting the sign-in…
      </div>

      <div v-else-if="phase.kind === 'device'" class="flex flex-col gap-4">
        <div
          class="flex items-center justify-between rounded-xl border border-line bg-surface px-4 py-3"
        >
          <span
            class="select-all font-mono text-[22px] tracking-[0.25em] text-fg-emphasis"
            >{{ phase.code }}</span
          >
          <IconButton
            :icon="copied ? Check : Copy"
            variant="ghost"
            :aria-label="copied ? 'Copied' : 'Copy code'"
            @click="copy(phase.code)"
          />
        </div>
        <div class="flex flex-wrap items-center gap-3">
          <Button variant="primary" @click="emit('open', phase.url)">
            Open in Browser
            <ExternalLink :size="ICON_PX.in24" />
          </Button>
          <!-- For a web browser on another machine, or another profile: the link alone. -->
          <Tooltip :content="linkCopied ? 'Copied' : 'Copy the link to sign in from another browser or device'">
            <IconButton
              :icon="linkCopied ? Check : Link"
              variant="ghost"
              :aria-label="linkCopied ? 'Copied' : 'Copy link'"
              @click="copyLink(phase.url)"
            />
          </Tooltip>
          <span class="flex items-center gap-1.5 text-[12px] text-fg-subtle">
            <IndeterminateSpinner :size="ICON_PX.in20" />
            Waiting<template v-if="phase.expiresIn">
              · code expires in {{ phase.expiresIn }}</template
            >
          </span>
        </div>
      </div>

      <!-- The vendor's page and the code it shows: open or copy the link, paste the code. -->
      <div v-else-if="phase.kind === 'code'" class="flex flex-col gap-4">
        <div class="flex flex-wrap items-center gap-3">
          <Button @click="emit('open', phase.url)">
            Open in Browser
            <ExternalLink :size="ICON_PX.in24" />
          </Button>
          <!-- For a web browser on another machine, or another profile: the link alone. -->
          <Tooltip :content="linkCopied ? 'Copied' : 'Copy the link to sign in from another browser or device'">
            <IconButton
              :icon="linkCopied ? Check : Link"
              variant="ghost"
              :aria-label="linkCopied ? 'Copied' : 'Copy link'"
              @click="copyLink(phase.url)"
            />
          </Tooltip>
        </div>
        <div class="flex items-center gap-2">
          <TextInput
            v-model="code"
            trim
            placeholder="Paste the code"
            aria-label="Code"
            class="flex-1"
            :disabled="phase.submitted"
            @keydown.enter="submitCode"
          />
          <Button
            variant="primary"
            :disabled="!code || phase.submitted"
            @click="submitCode"
            >Continue</Button
          >
        </div>
        <span class="flex items-center gap-1.5 text-[12px] text-fg-subtle">
          <IndeterminateSpinner :size="ICON_PX.in20" />
          <template v-if="phase.submitted">Signing in…</template>
          <template v-else
            >Waiting for the code<template v-if="phase.expiresIn">
              · expires in {{ phase.expiresIn }}</template
            ></template
          >
        </span>
      </div>

      <div
        v-else-if="phase.kind === 'done'"
        class="flex items-center gap-2 py-2 text-chrome text-fg"
      >
        <Check :size="ICON_PX.in28" class="text-on-success" />
        {{ phase.account }}
      </div>

      <InlineError v-else :message="phase.message" />

      <div class="flex justify-end gap-2">
        <Button
          v-if="phase.kind === 'done'"
          variant="primary"
          @click="emit('close')"
          >Done</Button
        >
        <template v-else>
          <Button @click="emit('close')">Cancel</Button>
          <Button v-if="phase.kind === 'failed'" @click="emit('retry')"
            >Try Again</Button
          >
        </template>
      </div>
    </div>
  </Dialog>
</template>
