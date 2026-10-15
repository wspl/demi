<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, shallowRef, watch, watchEffect } from 'vue'
import MediaViewer from '@demicodes/web-ui/files/MediaViewer.vue'
import SharedConversation from '@demicodes/web-ui/share/SharedConversation.vue'
import SharedLinkMissing from '@demicodes/web-ui/share/SharedLinkMissing.vue'
import { t } from '@demicodes/web-ui/share/locales/text'
import { provideBlobUrl } from '@demicodes/web-ui/agent/media-source'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'
import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import { lookupAttachment } from '../api/attachments'
import type { SharedConversation as Share } from '../api/generated/web-api'
import type { HistoryHolder } from '../conversation/history-reads'
import { agentFragment, shownAgent } from './address'
import { createShareReads, readShare, shareBlobUrl, sharePath, shareSnapshot } from './reads'

/**
 * A share's page at `/s/<token>` (`sharing.md`): what the share shows above
 * its snapshot, and the snapshot's transcripts, each read from its latest
 * page back as the reader scrolls, through the share's routes alone.
 */
const props = defineProps<{ token: string }>()

const lifetime = new AbortController()
onBeforeUnmount(() => lifetime.abort())
const reads = createShareReads(() => lifetime.signal)
const blobUrl = shareBlobUrl(props.token)
provideBlobUrl(blobUrl)
const mediaViewer = provideMediaViewer()

/** What the link names: still asking, no share, the share, or asking failed. */
const share = shallowRef<{ state: 'loading' } | { state: 'missing' } | { state: 'found'; share: Share } | { state: 'failed'; error: ShownError }>({ state: 'loading' })
const snapshot = ref<HistoryHolder | null>(null)
/** The transcripts whose latest page failed, by node ('' for the conversation's own), with why. */
const failed = reactive(new Map<string, ShownError>())

const agent = ref(shownAgent(location.hash))
const follow = () => {
  agent.value = shownAgent(location.hash)
}
window.addEventListener('popstate', follow)
onBeforeUnmount(() => window.removeEventListener('popstate', follow))

/** A click on an `agent:` link, or Back in a subagent's header: a new entry of the browser's history. */
function openAgent(number: number | null): void {
  if (number === agent.value) {
    return
  }
  history.pushState(null, '', `${location.pathname}${location.search}${agentFragment(number)}`)
  agent.value = number
}

const node = computed(() => (agent.value === null ? null : String(agent.value)))

async function load(): Promise<void> {
  share.value = { state: 'loading' }
  failed.clear()
  try {
    const found = await readShare(props.token, lifetime.signal)
    if (found === null) {
      share.value = { state: 'missing' }
      return
    }
    snapshot.value = shareSnapshot(props.token, found.agents)
    share.value = { state: 'found', share: found }
  } catch (error) {
    if (!lifetime.signal.aborted) {
      share.value = { state: 'failed', error: shownError(error) }
    }
  }
}
void load()

/** Reads the shown transcript's latest page when the page holds none of it. */
function readShown(): void {
  const held = snapshot.value
  const at = node.value
  if (!held || shownHistory.value.windows.length > 0 || failed.has(at ?? '')) {
    return
  }
  reads.readLatest(held, at).catch((error: unknown) => {
    if (!lifetime.signal.aborted) {
      failed.set(at ?? '', shownError(error))
    }
  })
}

const shownHistory = computed(() => {
  const held = snapshot.value
  if (!held) {
    return EMPTY_TRANSCRIPT
  }
  return node.value === null ? held.history : held.subagents.find((candidate) => candidate.id === node.value)?.history ?? EMPTY_TRANSCRIPT
})
watch([snapshot, node], readShown, { immediate: true })

const load_ = computed<ListLoad>(() => {
  if (share.value.state === 'failed' || failed.has(node.value ?? '')) {
    return 'failed'
  }
  return shownHistory.value.windows.length > 0 ? 'ready' : 'loading'
})
const loadError = computed(() => share.value.state === 'failed' ? share.value.error : failed.get(node.value ?? '') ?? null)

function retry(): void {
  if (share.value.state === 'failed') {
    void load()
    return
  }
  failed.delete(node.value ?? '')
  readShown()
}

function readBeside(side: 'before' | 'after'): void {
  const held = snapshot.value
  if (!held) {
    return
  }
  const read = side === 'before' ? reads.readBefore(held, node.value) : reads.readAfter(held, node.value)
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
    :agents="share.state === 'found' ? share.share.agents : []"
    :agent="agent"
    :history="shownHistory"
    :summaries="agent === null ? snapshot?.summaries : undefined"
    :load="load_"
    :load-error="loadError"
    :read-block="async (at, id) => { if (snapshot) await reads.readWhole(snapshot, at, id) }"
    :attachment="(id) => lookupAttachment(attachments, id)"
    @open-agent="openAgent"
    @read-before="readBeside('before')"
    @read-after="readBeside('after')"
    @retry-load="retry"
  />
  <MediaViewer :viewer="mediaViewer" />
</template>
