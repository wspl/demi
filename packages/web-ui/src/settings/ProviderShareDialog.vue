<script setup lang="ts">
import { computed, ref } from 'vue'
import { Check, Copy, Plus } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import Dialog from '../ui/Dialog.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import TextInput from '../ui/TextInput.vue'
import { useClock } from '../composables/useClock'
import { formatUntil } from '../ui/dates'
import { shownError } from '../infra/errors'
import { ICON_PX } from '../ui/icon-metrics'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import type { Action, Operation } from '../ui/operations'
import type { MadeKeyPhase, ShareKeysPhase } from './provider-sharing'
import type { SettingsShareKey } from './types'

/**
 * Share Provider: the share key just made, in a read-only field, and the
 * provider's keys, newest first, each with Revoke.
 *
 * - A key's text is shown only by the dialog that made it, so the field is
 *   empty once the dialog reopens; New Key makes another there.
 * - While a key is made, Creating key… stands in the field with a spinner at
 *   its end; a failure says why under it, and Try Again takes Copy Key's place.
 * - A key used shows the server that used it last and when; one never used,
 *   when it expires. Revoking a used key asks first, since a server loses the
 *   provider; an unused one goes at once.
 * - Copy Key is the default button and turns to Copied after a copy, keeping
 *   its width.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The provider's name. */
  name: string
  keys: ShareKeysPhase
  made: MadeKeyPhase | null
  /** The key was just copied. */
  copied: boolean
  /** Revoke, returning the promise of its work. */
  onRevoke?: Action<[keyId: string]>
}>()

const emit = defineEmits<{
  close: []
  copy: []
  /** New Key, or Try Again after making one failed. */
  make: []
  reload: []
}>()

const now = useClock()
const title = computed(() => t('providers.share.title', { name: props.name }))
const key = computed(() => (props.made?.status === 'ready' ? props.made : null))
const failure = computed(() => (props.made?.status === 'failed' ? shownError(props.made.error) : null))
const listFailure = computed(() => (props.keys.status === 'failed' ? shownError(props.keys.error) : null))
const creating = computed(() => props.made?.status === 'creating')

/** The used key whose Revoke asks first. */
const revoking = ref<SettingsShareKey | null>(null)
const confirmOpen = ref(false)
const revokeOf = (shareKey: SettingsShareKey | null): Operation => ({ action: 'revoke', target: `share-key:${shareKey?.id ?? ''}` })

function askRevoke(shareKey: SettingsShareKey): unknown {
  if (!shareKey.used) {
    return props.onRevoke?.(shareKey.id)
  }
  revoking.value = shareKey
  confirmOpen.value = true
  return undefined
}

async function revoke(): Promise<void> {
  const shareKey = revoking.value
  if (shareKey) {
    await props.onRevoke?.(shareKey.id)
  }
  confirmOpen.value = false
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="title" @close="emit('close')">
    <div class="flex flex-col gap-3 px-5 pt-5">
      <h3 class="select-none pr-8 text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      <p class="text-explanation leading-5 text-fg-body">{{ t('providers.share.who') }}</p>
      <!-- The field and why making a key failed take one cell, which keeps the
           taller one's height, so a failure moves nothing below it. -->
      <div class="grid">
        <TextInput
          class="col-start-1 row-start-1 self-start"
          :class="failure ? 'invisible' : ''"
          :model-value="key?.text ?? ''"
          :placeholder="creating ? t('providers.share.creating') : t('providers.share.empty')"
          :aria-label="t('providers.share.field')"
          :aria-busy="creating || undefined"
          readonly
          literal
          autocomplete="off"
          @focus="($event.target as HTMLInputElement).select()"
        >
          <template v-if="creating" #suffix>
            <IndeterminateSpinner :size="14" />
          </template>
        </TextInput>
        <InlineError
          class="col-start-1 row-start-1"
          :class="failure ? '' : 'invisible'"
          :message="failure?.message ?? t('providers.share.makeFailed')"
          :request-id="failure ? failure.requestId : 'req_0000000000000000'"
        />
      </div>
      <section class="flex flex-col gap-1.5" :aria-label="t('providers.share.list')">
        <h4 class="select-none text-[12px] font-medium text-fg-subtle">{{ t('providers.share.list') }}</h4>
        <!-- Room for three keys, so the dialog keeps its height while the list loads or changes. -->
        <div class="h-[132px] overflow-y-auto rounded-lg border border-line">
          <div v-if="keys.status === 'loading'" class="flex h-full items-center justify-center" role="status" :aria-label="shared('common.loading')">
            <IndeterminateSpinner :size="16" />
          </div>
          <div v-else-if="listFailure" class="flex h-full flex-col items-center justify-center gap-2 px-3">
            <InlineError :message="listFailure.message" :request-id="listFailure.requestId" />
            <Button size="sm" @click="emit('reload')">{{ shared('common.reload') }}</Button>
          </div>
          <p v-else-if="keys.status === 'ready' && !keys.keys.length" class="flex h-full select-none items-center justify-center text-[13px] text-fg-subtle">
            {{ t('providers.share.none') }}
          </p>
          <ul v-else-if="keys.status === 'ready'" class="divide-y divide-line">
            <li v-for="shareKey in keys.keys" :key="shareKey.id" class="flex min-h-11 items-center gap-3 px-3 py-1.5">
              <div class="flex min-w-0 flex-1 flex-col text-[12px] leading-4">
                <span class="truncate text-[13px] leading-5 text-fg-body">
                  {{ shareKey.used ? t('providers.share.usedBy', { server: shareKey.used.server }) : t('providers.share.unused') }}
                </span>
                <span class="text-fg-subtle">
                  <template v-if="shareKey.used">
                    {{ t('providers.share.lastUsed') }} <RelativeTime class="text-fg-muted" :timestamp="shareKey.used.at" />
                  </template>
                  <template v-else-if="shareKey.expiresAt">
                    {{ t('providers.share.expires', { when: formatUntil(shareKey.expiresAt, now) }) }}
                  </template>
                </span>
              </div>
              <Button
                size="sm"
                :aria-label="shareKey.used ? t('providers.share.revokeUsedLabel', { server: shareKey.used.server }) : t('providers.share.revokeUnusedLabel')"
                :operation="revokeOf(shareKey)"
                @click="askRevoke(shareKey)"
              >{{ shareKey.used ? t('providers.share.revokeAsk') : t('providers.share.revoke') }}</Button>
            </li>
          </ul>
        </div>
      </section>
      <!-- Inside the dialog, so it stacks on it rather than replacing it. -->
      <ConfirmDialog
        :is-open="confirmOpen"
        :overlay-store="overlayStore"
        :title="t('providers.share.revokeTitle', { server: revoking?.used?.server ?? '' })"
        :action="t('providers.share.revoke')"
        :operation="revokeOf(revoking)"
        @close="confirmOpen = false"
        @confirm="revoke"
      >
        <p>{{ t('providers.share.revokeBody', { name }) }}</p>
      </ConfirmDialog>
    </div>
    <template #footer-leading>
      <Button :disabled="creating" @click="emit('make')"><Plus :size="ICON_PX.in24" /> {{ t('providers.share.newKey') }}</Button>
    </template>
    <template #footer>
      <Button v-if="made?.status === 'failed'" variant="primary" @click="emit('make')">{{ t('providers.share.tryAgain') }}</Button>
      <Button v-else variant="primary" :disabled="!key" @click="emit('copy')">
        <component :is="copied ? Check : Copy" :size="14" />
        <!-- Both labels take one cell, so the button keeps the wider one's width. -->
        <span class="inline-grid">
          <span class="col-start-1 row-start-1" :class="copied ? 'invisible' : ''">{{ t('providers.share.copy') }}</span>
          <span class="col-start-1 row-start-1" :class="copied ? '' : 'invisible'">{{ shared('common.copied') }}</span>
        </span>
      </Button>
    </template>
  </Dialog>
</template>
