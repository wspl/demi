<script setup lang="ts">
import { computed, onBeforeUnmount, shallowRef, watchEffect } from 'vue'
import MediaViewer from '@demicodes/web-ui/files/MediaViewer.vue'
import SharedConversation from '@demicodes/web-ui/share/SharedConversation.vue'
import SharedLinkMissing from '@demicodes/web-ui/share/SharedLinkMissing.vue'
import type { SharedItem } from '@demicodes/web-ui/share/shared-items'
import { t } from '@demicodes/web-ui/share/locales/text'
import { provideBlobUrl } from '@demicodes/web-ui/agent/media-source'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'
import type { HeldTranscript } from '@demicodes/web-ui/transport/protocol'
import { lookupAttachment } from '../api/attachments'
import type { SharedConversation as Share } from '../api/generated/web-api'
import { EMPTY_SHARE, createShareReads, readShare, shareBlobUrl, sharePath } from './reads'

/**
 * A share's page at `/s/<token>` (`sharing.md`): what the share shows above
 * what it holds, and what it holds, read from its latest page back as the
 * reader scrolls, through the share's routes alone.
 */
const props = defineProps<{ token: string }>()

const lifetime = new AbortController()
onBeforeUnmount(() => lifetime.abort())
const history = shallowRef<HeldTranscript<SharedItem>>(EMPTY_SHARE)
const reads = createShareReads(props.token, () => lifetime.signal, () => history.value, (held) => {
  history.value = held
})
const blobUrl = shareBlobUrl(props.token)
provideBlobUrl(blobUrl)
const mediaViewer = provideMediaViewer()

/** What the link names: still asking, no share, the share, or asking failed. */
const share = shallowRef<{ state: 'loading' } | { state: 'missing' } | { state: 'found'; share: Share } | { state: 'failed'; error: ShownError }>({ state: 'loading' })
/** Why the latest page failed; null while it did not. */
const failed = shallowRef<ShownError | null>(null)

async function load(): Promise<void> {
  share.value = { state: 'loading' }
  failed.value = null
  try {
    const found = await readShare(props.token, lifetime.signal)
    if (found === null) {
      share.value = { state: 'missing' }
      return
    }
    share.value = { state: 'found', share: found }
    readLatest()
  } catch (error) {
    if (!lifetime.signal.aborted) {
      share.value = { state: 'failed', error: shownError(error) }
    }
  }
}
void load()

/** Reads the share's latest page, which the page shows first. */
function readLatest(): void {
  failed.value = null
  reads.readLatest().catch((error: unknown) => {
    if (!lifetime.signal.aborted) {
      failed.value = shownError(error)
    }
  })
}

const listLoad = computed<ListLoad>(() => {
  if (share.value.state === 'failed' || failed.value !== null) {
    return 'failed'
  }
  return history.value.windows.length > 0 ? 'ready' : 'loading'
})
const loadError = computed(() => share.value.state === 'failed' ? share.value.error : failed.value)

function retry(): void {
  if (share.value.state === 'failed') {
    void load()
    return
  }
  readLatest()
}

function readBeside(side: 'before' | 'after'): void {
  const read = side === 'before' ? reads.readBefore() : reads.readAfter()
  // A page that failed is read again as the reader scrolls back to its edge.
  read.catch((error: unknown) => console.warn('A page of the shared conversation was not read', error))
}

const attachments = { path: sharePath(props.token), blobUrl }
const title = computed(() => share.value.state === 'found' ? share.value.share.title : null)
watchEffect(() => {
  document.title = share.value.state === 'missing'
    ? t('share.missing')
    : title.value?.trim() || t('share.untitled')
})
</script>

<template>
  <SharedLinkMissing v-if="share.state === 'missing'" />
  <SharedConversation
    v-else
    :title="title"
    :history="history"
    :load="listLoad"
    :load-error="loadError"
    :attachment="(id) => lookupAttachment(attachments, id)"
    @read-before="readBeside('before')"
    @read-after="readBeside('after')"
    @retry-load="retry"
  />
  <MediaViewer :viewer="mediaViewer" />
</template>
