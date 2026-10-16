<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, onUnmounted, ref, watch } from 'vue'
import { timeLeft } from '@demicodes/utils'
import { Check, ExternalLink } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import CopyButton from '@demicodes/web-ui/ui/CopyButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
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

/**
 * The code this page has typed and not yet had taken: it empties when the
 * dialog opens or closes, the phase changes kind, or the sign-in takes this
 * page's code, never on what another page pastes or the refusal of its code.
 */
const code = ref('')
watch([() => props.isOpen, () => props.phase.kind], () => {
  code.value = ''
})
watch(
  () => (props.phase.kind === 'code' ? props.phase.taken : undefined),
  (taken) => {
    if (taken !== undefined) {
      code.value = ''
    }
  },
)

/** The page's clock, which ticks each second while the phase counts down to its expiry. */
const now = ref(Date.now())
let ticking: ReturnType<typeof setInterval> | null = null
const expiresAt = computed(() =>
  props.isOpen && (props.phase.kind === 'device' || props.phase.kind === 'code')
    ? props.phase.expiresAt ?? null
    : null,
)
watch(
  expiresAt,
  (deadline) => {
    if (deadline === null) {
      stopTicking()
      return
    }
    now.value = Date.now()
    ticking ??= setInterval(() => {
      now.value = Date.now()
    }, 1000)
  },
  { immediate: true },
)
function stopTicking(): void {
  if (ticking !== null) {
    clearInterval(ticking)
    ticking = null
  }
}
onUnmounted(stopTicking)
/** The time the sign-in has left, as `9:05`; null without an expiry or once it passed. */
const left = computed(() => (expiresAt.value === null ? null : timeLeft(expiresAt.value, now.value)))

function submitCode(): void {
  if (props.phase.kind === 'code' && !props.phase.submitted && code.value) {
    emit('submitCode', code.value)
  }
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="t('login.title', { vendor: vendorName })"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-5 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">
          {{ t('login.title', { vendor: vendorName }) }}
        </h3>
        <!-- While it starts, the body alone says so. -->
        <p
          v-if="phase.kind !== 'starting'"
          class="mt-0.5 text-explanation leading-5 text-fg-muted"
        >
          <template v-if="phase.kind === 'device'">{{ t('login.deviceHint') }}</template>
          <template v-else-if="phase.kind === 'code'">{{ t('login.codeHint') }}</template>
          <template v-else-if="phase.kind === 'done' && phase.active">{{ t('login.doneActive', { vendor: vendorName }) }}</template>
          <template v-else-if="phase.kind === 'done'">{{ t('login.doneKept', { vendor: vendorName }) }}</template>
          <template v-else>{{ t('login.failedHint') }}</template>
        </p>
      </header>

      <div
        v-if="phase.kind === 'starting'"
        class="flex items-center gap-2 py-4 text-chrome text-fg-muted"
      >
        <IndeterminateSpinner :size="ICON_PX.in24" />
        <template v-if="phase.stage === 'waiting_for_cloud'">{{ t('login.waitingCloud') }}</template>
        <template v-else-if="phase.stage === 'installing'">{{ t('login.installing') }}</template>
        <template v-else-if="phase.stage === 'starting'">{{ t('login.startingTool') }}</template>
        <template v-else>{{ t('login.contacting', { vendor: vendorName }) }}</template>
      </div>

      <div v-else-if="phase.kind === 'device'" class="flex flex-col gap-4">
        <div
          class="flex items-center justify-between rounded-xl border border-line bg-surface px-4 py-3"
        >
          <span
            class="select-all font-mono text-[22px] tracking-[0.25em] text-fg-emphasis"
            >{{ phase.code }}</span
          >
          <CopyButton :source="phase.code" :label="t('login.copyCode')" size="md" />
        </div>
        <div class="flex flex-wrap items-center gap-3">
          <Button variant="primary" @click="emit('open', phase.url)">
            {{ t('login.openInBrowser') }}
            <ExternalLink :size="ICON_PX.in24" />
          </Button>
          <!-- For a web browser on another machine, or another profile: the link alone. -->
          <CopyButton
            :source="phase.url"
            :label="t('login.copyLink')"
            :hint="t('login.copyLinkHint')"
            size="md"
          />
          <span class="flex items-center gap-1.5 text-detail text-fg-subtle">
            <IndeterminateSpinner :size="ICON_PX.in20" />
            {{ left ? t('login.waitingExpires', { left }) : t('login.waiting') }}
          </span>
        </div>
      </div>

      <!-- The vendor's page and the code it shows: open or copy the link, paste the code. -->
      <div v-else-if="phase.kind === 'code'" class="flex flex-col gap-4">
        <div class="flex flex-wrap items-center gap-3">
          <Button @click="emit('open', phase.url)">
            {{ t('login.openInBrowser') }}
            <ExternalLink :size="ICON_PX.in24" />
          </Button>
          <!-- For a web browser on another machine, or another profile: the link alone. -->
          <CopyButton
            :source="phase.url"
            :label="t('login.copyLink')"
            :hint="t('login.copyLinkHint')"
            size="md"
          />
        </div>
        <div class="flex items-center gap-2">
          <TextInput
            v-model="code"
            trim
            :placeholder="t('login.paste')"
            :aria-label="t('login.code')"
            class="flex-1"
            :disabled="phase.submitted"
            @keydown.enter="submitCode"
          />
          <Button
            variant="primary"
            :disabled="!code || phase.submitted"
            @click="submitCode"
            >{{ uiText('common.continue') }}</Button
          >
        </div>
        <!-- The vendor's own words for a code it refused; the sign-in waits for another. -->
        <InlineError v-if="phase.error && !phase.submitted" :message="phase.error" />
        <span class="flex items-center gap-1.5 text-detail text-fg-subtle">
          <IndeterminateSpinner :size="ICON_PX.in20" />
          <template v-if="phase.submitted">{{ t('login.signingIn') }}</template>
          <template v-else>{{ left ? t('login.waitingCodeExpires', { left }) : t('login.waitingCode') }}</template>
        </span>
      </div>

      <div
        v-else-if="phase.kind === 'done'"
        class="flex items-center gap-2 py-2 text-chrome text-fg"
      >
        <Check :size="ICON_PX.in28" class="text-on-success" />
        {{ phase.account }}
      </div>

      <InlineError v-else :message="phase.message" :request-id="phase.requestId" />
    </div>
    <template #footer>
      <Button
        v-if="phase.kind === 'done'"
        variant="primary"
        @click="emit('close')"
        >{{ uiText('common.done') }}</Button
      >
      <template v-else>
        <!-- A failed sign-in has nothing left to cancel. -->
        <Button @click="emit('close')">{{
          phase.kind === 'failed' ? uiText('common.close') : uiText('common.cancel')
        }}</Button>
        <Button v-if="phase.kind === 'failed'" @click="emit('retry')"
          >{{ uiText('common.retry') }}</Button
        >
      </template>
    </template>
  </Dialog>
</template>
