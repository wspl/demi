<script setup lang="ts">
import Toast from '@demicodes/web-ui/ui/Toast.vue'
import { errorFacts } from '@demicodes/web-ui/agent/error-detail'
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

/** A restore the gallery simulates: it fails again after a moment, as a backend still down would. */
function retryRestore(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 1500))
}

const usageLimitPayload = JSON.stringify({
  type: 'error',
  error: {
    type: 'usage_limit_reached',
    message: 'The usage limit has been reached',
    plan_type: 'pro',
    resets_at: 1790062659,
    resets_in_seconds: 321250,
  },
  status_code: 429,
  headers: {
    'X-Codex-Plan-Type': 'pro',
    'X-Codex-Primary-Used-Percent': '100',
    'X-Codex-Primary-Window-Minutes': '10080',
  },
}, null, 2)
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="ErrorNotice · a Failure in the Conversation"
      note="One tinted bar, full width where it sits. The first line is what the source said, never Demi’s reading of it: a plain sentence is the whole record; a message that wraps a vendor’s JSON body leads with the sentence inside it and keeps the full text below; only a message with no sentence to lead with gets the neutral line. Then, when the vendor names it, the moment it works again; the status, Demi’s own code and the request ids stay in the copied report. The provider response folds open under a chevron, whole, exactly as it arrived. Copy takes the whole report. A turn’s failure carries no button: Resume sits in the dock above the composer. With one line the controls sit centred; with facts they hold the first line."
    >
      <GallerySpecimen wide variant="A plain sentence is the whole record · when it lifts leads the facts · the vendor payload behind a disclosure">
        <ErrorNotice
          label="The usage limit has been reached"
          :facts="errorFacts('2026-09-22T07:37:39.000Z')"
          :raw="usageLimitPayload"
          copy-text="The usage limit has been reached"
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="A wrapped vendor body · its sentence first, the full text below">
        <ErrorNotice
          label="Insufficient balance. Manage your billing here: https://vendor.example/billing"
          detail='OpenAI API request failed with HTTP 401: {"type":"error","error":{"type":"CreditsError","message":"Insufficient balance. Manage your billing here: https://vendor.example/billing"}}'
          copy-text="OpenAI API request failed with HTTP 401"
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="No sentence to lead with · the neutral line over the text">
        <ErrorNotice
          label="The turn failed"
          detail="Anthropic API request failed with HTTP 429: This request would exceed the rate limit of 50 requests per minute for your organization. Retry after 12 seconds. Your current usage is 50 requests in the last 60 seconds across all models."
          copy-text="Anthropic API request failed with HTTP 429"
        />
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="RegionStatus · Content Cannot Be Shown"
      note="One pane for every region: the session, a settings list, the sidebar, a dialog body. A glyph for the kind, one sentence, the reason, and Retry, which returns the region to loading in the same frame, before the retried work answers; only a new failure brings the reason back. This specimen’s Retry fails again after a moment, as a backend still down would."
    >
      <div class="grid gap-6 lg:grid-cols-3">
        <GallerySpecimen wide variant="Failed">
          <div class="rounded-xl border border-line bg-surface">
            <!-- Retry loads again and, as the gallery has no backend, fails the same way after a moment. -->
            <RegionStatus
              status="failed"
              label="Couldn’t load this conversation."
              loading-label="Loading conversation…"
              detail="Could not load the transcript: HTTP 502 Bad Gateway"
              :on-retry="retryRestore"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Loading">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus status="loading" label="Loading conversation…" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Empty">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus status="note" label="No messages yet." />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="InlineError · a Form Rejected Its Own Input"
      note="A line under the fields, in their width. The form’s submit is the retry, so the line carries none."
    >
      <GallerySpecimen wide variant="Under a field">
        <InlineError message="The email or password is incorrect." />
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="Composer Notices · a State, Not a Failure"
      note="SessionNoticeBar replaces the composer input for a state the reader chose or can leave. The model catalog failing to load is the one composer failure, told beside the model chip."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Archived">
          <SessionNoticeBar
            label="This conversation is archived."
            action="Restore Conversation"
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
      title="Toast · the Request Itself Failed"
      note="A save that did not reach the server, a fork or edit the server refused, a revoke, a background refresh: the reader cannot fix these on the page, so the page shows nothing inline. Title, and a message only when it adds a fact. Success is silent."
    >
      <GallerySpecimen wide variant="Danger">
        <Toast
          title="Could Not Save the Provider"
          message="HTTP 503: the endpoint is unavailable."
          tone="danger"
        />
      </GallerySpecimen>
    </GallerySection>
  </div>
</template>
