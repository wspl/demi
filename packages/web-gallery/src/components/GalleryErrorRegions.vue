<script setup lang="ts">
import Toast from '@demicodes/web-ui/ui/Toast.vue'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import RegionStatus from '@demicodes/web-ui/ui/RegionStatus.vue'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import SessionNoticeBar from '@demicodes/web-ui/agent/SessionNoticeBar.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * The error primitives, each pinned in every variant it has, and the
 * product's toasts. This is the reference for the rest of the Errors page:
 * a failure is shown by exactly one of these, chosen by where it belongs.
 */
const regions = [
  { label: 'Devices', error: "Couldn't load devices." },
  { label: 'Providers', error: "Couldn't load providers." },
]
/** Product toasts as the product raises them: a title, and a message only when it adds a fact. */
const toasts = [
  { origin: 'settings/providers · save', title: 'Could not save the provider', message: 'HTTP 503: the endpoint is unavailable.' },
  { origin: 'settings/account · rename', title: 'Could not save the display name', message: 'Failed to fetch' },
  { origin: 'App · signOut', title: 'Could not sign out', message: 'Failed to fetch' },
  { origin: 'settings/devices · revoke', title: 'Could not revoke device', message: 'HTTP 409: the device is still paired.' },
  { origin: 'state/preferences · save', title: 'Could not update settings', message: 'Failed to fetch' },
  { origin: 'conversation store · notice', title: 'This conversation is still running.' },
  { origin: 'message fork · failed', title: 'Could not fork from this message', message: 'The server did not create the conversation.' },
  { origin: 'message edit · refused', title: 'The edit was not accepted', message: 'The conversation changed after this message.' },
  { origin: 'composer · rejected drop', title: 'Unsupported attachment type', message: 'archive.tar.gz, notes.rtf' },
  { origin: 'composer · edit attachment unreadable', title: "Couldn't attach", message: 'reference.png' },
]
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="ErrorNotice · a failure in the conversation"
      note="One tinted bar, full width where it sits: a sentence, the upstream message, the facts, Copy for a support thread, and Retry only where the failure is the tail of the conversation."
    >
      <GallerySpecimen wide variant="Turn failed · tail, with facts, Copy and Retry">
        <ErrorNotice
          label="Rate limited by the provider"
          detail="Anthropic API request failed with HTTP 429: This request would exceed the rate limit of 50 requests per minute for your organization. Retry after 12 seconds."
          :facts="['HTTP 429', 'rate_limit', 'req_01J8Y3Q6ZKX4']"
          copy-text="Anthropic API request failed with HTTP 429"
          action="Retry"
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="Turn failed · history, no Retry">
        <ErrorNotice
          label="The provider is overloaded or unreachable"
          detail="Overloaded. The provider could not accept the request."
          :facts="['HTTP 529', 'overloaded', 'req_01J8Y3Q6ZKX5']"
          copy-text="Overloaded."
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="Session failed · history in memory">
        <ErrorNotice
          label="Agent socket failed to connect: ECONNREFUSED 127.0.0.1:18911"
          action="Retry"
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="Request refused · nothing to retry">
        <ErrorNotice label="The request was refused: another view holds this conversation." />
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="RegionStatus · content cannot be shown"
      note="One pane for every region: the session, a settings list, the sidebar, a dialog body. Failed regions carry the reason and Retry; Retry returns the region to loading. Loading and empty regions only a sentence."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Failed · with reason">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus
              failed
              label="Couldn't load this conversation."
              detail="Agent socket failed to connect: ECONNREFUSED 127.0.0.1:18911"
              action="Retry"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Failed · no reason">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus failed label="Couldn't load conversations." action="Retry" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Loading">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus busy label="Loading conversation" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Empty">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus label="No messages yet. Start a conversation." />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Sidebar list failed · sidebar width">
          <div class="w-60 rounded-xl border border-line bg-surface-base">
            <RegionStatus
              class="min-h-40"
              failed
              label="Couldn't load conversations."
              action="Retry"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen
          v-for="region in regions"
          :key="region.label"
          wide
          :variant="`AsyncRegion in a settings card · ${region.label}`"
        >
          <div class="@container rounded-xl border border-line bg-surface px-8 py-8">
            <div class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float">
              <AsyncRegion state="failed" :error="region.error">
                <p class="p-6 text-chrome text-fg-muted">{{ region.label }} loaded.</p>
              </AsyncRegion>
            </div>
          </div>
        </GallerySpecimen>
      </div>
      <GallerySpecimen wide variant="Archived page · failed initial read">
        <div class="@container rounded-xl border border-line bg-surface px-8 py-8">
          <SettingsArchived load="failed" :conversations="[]" />
        </div>
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="InlineError · a form rejected its own input"
      note="A line under the fields, in their width. The form's submit is the retry, so the line carries none. Long text wraps."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="One line">
          <InlineError message="The email or password is incorrect." />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Dismissible">
          <InlineError
            message="This folder name is already taken. Choose another name or open the existing folder."
            dismissible
          />
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Composer notices · a state, not a failure"
      note="SessionNoticeBar replaces the composer input for a state the reader chose or can leave. The model catalog failing to load is the one composer failure, told beside the model chip."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Archived">
          <SessionNoticeBar
            label="This conversation is archived."
            action="Restore conversation"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="No models">
          <SessionNoticeBar
            label="No models available."
            action="Configure models"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Model catalog failed · beside the chip">
          <div class="rounded-xl border border-line bg-surface p-2">
            <ModelSelector load="failed" :providers="[]" :models="{}" />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Toast · the request itself failed"
      note="A save that did not reach the server, a fork or edit the server refused, a revoke, a background refresh: the reader cannot fix these on the page, so the page shows nothing inline. Success is silent. Each toast is shown as the product raises it."
    >
      <div class="grid items-start gap-6 lg:grid-cols-2">
        <GallerySpecimen
          v-for="toast in toasts"
          :key="toast.title"
          wide
          :variant="toast.origin"
        >
          <Toast :title="toast.title" :message="toast.message" tone="danger" />
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>
