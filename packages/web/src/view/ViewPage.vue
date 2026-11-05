<script setup lang="ts">
import { onBeforeUnmount, onMounted, shallowRef } from 'vue'
import ViewTab from '@demicodes/plugin-browser/live/ViewTab.vue'
import page from '@demicodes/plugin-browser'
import RegionStatus from '@demicodes/web-ui/ui/RegionStatus.vue'
import ViewLinkGone from '@demicodes/web-ui/view/ViewLinkGone.vue'
import { t } from '@demicodes/web-ui/ui/locales/text'
import { t as shared } from '@demicodes/web-ui/locales/text'
import { PageScope, providePageHost } from '@demicodes/web-ui/plugins/page'
import { shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { viewStreamOf } from '@demicodes/web-ui/transport/user-stream'
import { ApiError, apiRequest, readResponse } from '../api/client'
import { viewLinkSchema } from '../api/generated/web-api'
import { accountlessPageHost } from '../plugins/accountless-host'
import { finalEnd, tellParent, type ViewEnd } from './parent'
import { readViewed } from './conversation'
import ConversationViewPage from './ConversationViewPage.vue'
import type { ViewedConversation } from '../api/generated/web-api'

/**
 * A view link's page: the one tab of a conversation's browser the link
 * names, and nothing else, or the conversation itself, which the page only
 * watches (`crates/backend-http/src/view_links.rs`). It reads the link, shows
 * the tab with the browser's own page or the conversation with its own
 * page's parts, and tells the window that frames it once it shows and once
 * the view ended for good: the link expired, was removed or never was, or the
 * tab closed.
 */
const props = defineProps<{ token: string }>()

providePageHost(accountlessPageHost('A view link’s page', { pages: () => [], blockDocument: () => '' }))
const open = viewStreamOf(props.token)

const view = shallowRef<
  | { state: 'loading' }
  | { state: 'gone' }
  | { state: 'failed'; error: ShownError }
  | { state: 'found'; tab: string; readOnly: boolean }
  | { state: 'watching'; viewed: ViewedConversation; expiresAt: string | null }
>({ state: 'loading' })
let reading: AbortController | null = null

async function read(): Promise<void> {
  reading?.abort()
  const controller = new AbortController()
  reading = controller
  view.value = { state: 'loading' }
  try {
    const found = await readResponse(await apiRequest(`/views/${encodeURIComponent(props.token)}`, { signal: controller.signal }), viewLinkSchema)
    if (found.kind === 'tab') {
      view.value = { state: 'found', tab: found.tab, readOnly: found.readOnly }
      return
    }
    const viewed = await readViewed(props.token, controller.signal)
    view.value = { state: 'watching', viewed, expiresAt: found.expiresAt }
  } catch (error) {
    if (controller.signal.aborted) {
      return
    }
    if (error instanceof ApiError && error.code === 'view_not_found') {
      end('not_found')
      return
    }
    view.value = { state: 'failed', error: shownError(error) }
  }
}

/**
 * The conversation's connection could not be made: the link may have gone. A
 * link that no longer opens ends the page, as removed, or as expired once its
 * time is past; any other answer leaves the connection to try again.
 */
async function lost(): Promise<void> {
  if (view.value.state !== 'watching') {
    return
  }
  const { expiresAt } = view.value
  try {
    await apiRequest(`/views/${encodeURIComponent(props.token)}`)
  } catch (error) {
    if (error instanceof ApiError && error.code === 'view_not_found' && view.value.state === 'watching') {
      end(expiresAt !== null && Date.parse(expiresAt) <= Date.now() ? 'expired' : 'removed')
    }
  }
}

function end(reason: ViewEnd): void {
  view.value = { state: 'gone' }
  tellParent({ type: 'demi.view', state: 'ended', reason })
}

/** A stream that ended for good ends the view; any other end opens again. */
function ended(reason: string): void {
  const final = finalEnd(reason)
  if (final !== null) {
    end(final)
  }
}

onMounted(() => void read())
onBeforeUnmount(() => reading?.abort())
</script>

<template>
  <main class="h-full bg-surface" data-view-page>
    <ViewLinkGone v-if="view.state === 'gone'" />
    <RegionStatus v-else-if="view.state === 'loading'" class="h-full" status="loading" :label="shared('common.loading')" />
    <RegionStatus
      v-else-if="view.state === 'failed'"
      class="h-full"
      status="failed"
      :label="t('asyncRegion.error')"
      :detail="view.error.message"
      :request-id="view.error.requestId"
      :on-retry="read"
    />
    <ConversationViewPage v-else-if="view.state === 'watching'" :token="token" :viewed="view.viewed" @lost="lost" @retry="read" />
    <!-- The tab tells of its tab's close; this page says it once, as the tab's end. -->
    <PageScope
      v-else
      :page="page"
      :component="ViewTab"
      :props="{
        open,
        tab: view.tab,
        readOnly: view.readOnly,
        onLive: () => tellParent({ type: 'demi.view', state: 'live' }),
        onEnded: ended,
        onClosed: () => tellParent({ type: 'demi.view', state: 'ended', reason: 'tab_closed' }),
      }"
    />
  </main>
</template>
