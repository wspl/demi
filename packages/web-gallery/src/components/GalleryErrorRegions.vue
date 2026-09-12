<script setup lang="ts">
import Toast from '@demicodes/web-ui/ui/Toast.vue'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import RegionStatus from '@demicodes/web-ui/ui/RegionStatus.vue'
import SessionNoticeBar from '@demicodes/web-ui/agent/SessionNoticeBar.vue'
import GalleryComposer from './GalleryComposer.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Each error surface once, in its fullest form. The other pages show where
 * each one sits; this page is the reference for what each one looks like.
 */
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="ErrorNotice · a failure in the conversation"
      note="One tinted bar, full width where it sits: a sentence, the upstream message, the facts, Copy for a support thread, and Retry only where the failure is the tail of the conversation. With one line the controls sit centred; with facts they hold the first line."
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
    </GallerySection>
    <GallerySection
      title="RegionStatus · content cannot be shown"
      note="One pane for every region: the session, a settings list, the sidebar, a dialog body. A glyph for the kind, one sentence, the reason, and Retry, which returns the region to loading."
    >
      <div class="grid gap-6 lg:grid-cols-3">
        <GallerySpecimen wide variant="Failed">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus
              failed
              label="Couldn't load this conversation."
              detail="Could not load the transcript: HTTP 502 Bad Gateway"
              action="Retry"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Loading">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus busy label="Loading conversation" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Empty">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus label="No messages yet." />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="InlineError · a form rejected its own input"
      note="A line under the fields, in their width. The form's submit is the retry, so the line carries none."
    >
      <GallerySpecimen wide variant="Under a field">
        <InlineError message="The email or password is incorrect." />
      </GallerySpecimen>
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
        <GallerySpecimen wide variant="Model catalog failed · beside the chip">
          <GalleryComposer
            placeholder="Ask Demi…"
            model-load="failed"
          />
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Toast · the request itself failed"
      note="A save that did not reach the server, a fork or edit the server refused, a revoke, a background refresh: the reader cannot fix these on the page, so the page shows nothing inline. Title, and a message only when it adds a fact. Success is silent."
    >
      <GallerySpecimen wide variant="Danger">
        <Toast
          title="Could not save the provider"
          message="HTTP 503: the endpoint is unavailable."
          tone="danger"
        />
      </GallerySpecimen>
    </GallerySection>
  </div>
</template>
