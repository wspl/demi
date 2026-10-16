<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { CircleCheck } from '@lucide/vue'
import QRCode from 'qrcode'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import { timeLeft } from '@demicodes/utils'
import Dialog from '../ui/Dialog.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'
import {
  HANDOFF_WORDING,
  type HandoffCodeView,
  type HandoffHost,
  type HandoffWording,
} from './handoff'
import VerifyForm from './VerifyForm.vue'
import { useVerification } from './verification'

/**
 * Hand Off to Your Phone (`users.md` § User system). Opening it asks the
 * host for a code; when the backend wants the user to confirm it is them
 * first, the confirmation step comes before it: the password, red with
 * Required or the host's sentence under it as other forms are, or Use
 * Passkey where the host offers one. The code step shows the code's URL as
 * a QR code and the time left; an expired one offers Show New Code, which
 * asks again. Once the host names the browser the code was handed off to,
 * the dialog says so and closes by itself a moment later. `wording` words
 * it for who shows the code and who scans it; a `code-lead` slot puts
 * something above the QR code, such as Add User's first password.
 */
const props = withDefaults(defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  host: HandoffHost
  /** The account's email, which the password manager files the password under. */
  email: string
  wording?: HandoffWording
}>(), {
  wording: () => HANDOFF_WORDING,
})

const emit = defineEmits<{
  close: []
}>()

/** How long the dialog says it handed off to the phone before it closes. */
const CLOSE_AFTER_MS = 1600

type Step =
  | { kind: 'starting' }
  | { kind: 'verify' }
  | { kind: 'code'; code: HandoffCodeView; svg: string }
  | ({ kind: 'failed' } & ShownError)

const step = ref<Step>({ kind: 'starting' })
const verification = useVerification(() => props.host, () => start())
const now = ref(Date.now())
let ticking: ReturnType<typeof setInterval> | null = null
let closing: ReturnType<typeof setTimeout> | null = null
/** Each opening's requests; one answered after the dialog closed or asked again is dropped. */
let attempt = 0

function stopTimers(): void {
  if (ticking !== null) {
    clearInterval(ticking)
    ticking = null
  }
  if (closing !== null) {
    clearTimeout(closing)
    closing = null
  }
}

async function start(): Promise<void> {
  const mine = ++attempt
  step.value = { kind: 'starting' }
  const started = await props.host.makeCode()
  if (mine !== attempt) {
    return
  }
  switch (started.kind) {
    case 'code': {
      // The modules stay dark on light whatever the theme: a camera reads
      // an inverted code poorly.
      const svg = await QRCode.toString(started.code.url, { type: 'svg', margin: 0, errorCorrectionLevel: 'M' })
      if (mine !== attempt) {
        return
      }
      now.value = Date.now()
      step.value = { kind: 'code', code: started.code, svg }
      ticking ??= setInterval(() => {
        now.value = Date.now()
      }, 1000)
      break
    }
    case 'verify':
      verification.reset()
      step.value = { kind: 'verify' }
      break
    case 'failed':
      step.value = started
      break
  }
}

watch(
  () => props.isOpen,
  (open) => {
    attempt += 1
    stopTimers()
    if (open) {
      void start()
    }
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  attempt += 1
  stopTimers()
})
const left = computed(() => (step.value.kind === 'code' ? timeLeft(step.value.code.deadline, now.value) : null))
const handedOffTo = computed(() =>
  step.value.kind === 'code' ? props.host.handedOffTo(step.value.code.id) : null,
)
watch(left, (value) => {
  // An expired code needs no clock until a new one shows.
  if (value === null && ticking !== null) {
    clearInterval(ticking)
    ticking = null
  }
})
watch(handedOffTo, (name) => {
  if (name !== null && closing === null) {
    if (ticking !== null) {
      clearInterval(ticking)
      ticking = null
    }
    closing = setTimeout(() => {
      closing = null
      emit('close')
    }, CLOSE_AFTER_MS)
  }
})
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="wording.title"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ wording.title }}</h3>
        <p v-if="step.kind === 'verify'" class="mt-0.5 text-explanation leading-5 text-fg-muted">
          {{ wording.confirm }}
        </p>
      </header>
      <div v-if="step.kind === 'starting'" class="flex h-40 items-center justify-center" role="status" :aria-label="t('handoff.making')">
        <IndeterminateSpinner />
      </div>
      <template v-else-if="step.kind === 'verify'">
        <VerifyForm :verification="verification" :email="email" />
      </template>
      <template v-else-if="step.kind === 'code'">
        <slot name="code-lead" />
        <div class="flex flex-col items-center gap-3 pb-1">
          <!-- The QR library's own SVG of the code: paths only, never the URL's text. -->
          <div
            v-if="handedOffTo === null"
            class="flex size-48 items-center justify-center rounded-xl bg-white p-3 [&>svg]:size-full"
            :class="{ 'opacity-15': left === null }"
            role="img"
            :aria-label="t('handoff.qr')"
            :data-url="step.code.url"
            v-html="step.svg"
          />
          <div v-else class="flex size-48 flex-col items-center justify-center gap-3 text-center" role="status">
            <CircleCheck :size="40" class="text-on-success" />
            <p class="text-explanation leading-5 text-fg">{{ wording.done(handedOffTo) }}</p>
          </div>
          <template v-if="handedOffTo === null">
            <p v-if="left !== null" class="select-none text-center text-explanation leading-5 text-fg-muted">
              {{ wording.scan }}
              <span class="block text-detail tabular-nums text-fg-subtle">{{ t('handoff.expires', { left }) }}</span>
            </p>
            <p v-else class="select-none text-center text-explanation leading-5 text-fg-muted">{{ t('handoff.expired') }}</p>
          </template>
        </div>
        </template>
      <InlineError v-else :message="step.message" :request-id="step.requestId" />
    </div>
    <template #footer>
      <template v-if="step.kind === 'verify'">
        <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
        <Button
          v-if="host.verifyPasskey"
          :operation="verification.withPasskey"
          @click="verification.usePasskey()"
        >
          {{ t('handoff.usePasskey') }}
        </Button>
        <Button
          variant="primary"
          :operation="verification.withPassword"
          @click="verification.submitPassword()"
        >
          {{ uiText('common.continue') }}
        </Button>
      </template>
      <template v-else-if="step.kind === 'code' && left === null && handedOffTo === null">
        <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
        <Button variant="primary" @click="start">{{ t('handoff.showNew') }}</Button>
      </template>
      <template v-else-if="step.kind === 'failed'">
        <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
        <Button variant="primary" @click="start">{{ uiText('common.retry') }}</Button>
      </template>
      <Button v-else variant="primary" @click="emit('close')">{{ uiText('common.done') }}</Button>
    </template>
  </Dialog>
</template>
