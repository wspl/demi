<script setup lang="ts">
import Toast from '@demicodes/web-ui/ui/Toast.vue'
import { failureHeadline } from '@demicodes/web-ui/agent/error-detail'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import RegionStatus from '@demicodes/web-ui/ui/RegionStatus.vue'
import SessionNoticeBar from '@demicodes/web-ui/agent/SessionNoticeBar.vue'
import { t as agentText } from '@demicodes/web-ui/agent/locales/text'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { productWould } from '../product-would'
import { refusalFailure } from '../fixtures/request-failures'
import { reportError } from '@demicodes/web-ui/infra/errors'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ref } from 'vue'

/**
 * Each error surface once, in its fullest form. The other pages show where
 * each one sits; this page is the reference for what each one looks like.
 */

/** A refused request, as the product shows one: the catalogue's words for its code and the ID the backend answered with. */
const refused = refusalFailure('provider_busy')
/** A refusal that carries a value its words need: an upload over the size an attachment may have. */
const tooLarge = refusalFailure('too_large', { limitBytes: 26_214_400 })
/** The transcript read the server could not do, in the page's words. */
const unreadable = refusalFailure('conversation_unreadable')
/** The moment the specimens' failures are read at, and the one a usage limit lifts at. */
const now = new Date('2026-09-20T12:00:00.000Z')
const lifts = '2026-09-22T07:37:39.000Z'
const creditsResponse = 'OpenAI API request failed with HTTP 401: {"type":"error","error":{"type":"CreditsError","message":"Insufficient balance. Manage your billing here: https://vendor.example/billing"}}'
const rateResponse = 'Anthropic API request failed with HTTP 429: This request would exceed the rate limit of 50 requests per minute for your organization. Retry after 12 seconds.'
/** Whether the failure toast specimen is still shown; Close hides it, Show Again brings it back. */
const toastShown = ref(true)

/**
 * A failure the page found itself, as the product meets one: an answer
 * whose value did not read. The product reports it to the server's log and
 * the toast names the report's ID once it is in; the gallery's reporter
 * answers with a simulated ID.
 */
function failOnThePage(): void {
  reportError('Could Not Read the Plugin’s Data', new Error('Invalid server response: Invalid input: expected string, received number'), {
    userVisible: true,
    detail: '✖ Invalid input: expected string, received number\n  → at sections[0].title',
  })
}

/** A restore the gallery simulates: it fails again after a moment, as a backend that still fails the read would. */
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
      note="One tinted bar, full width where it sits. The headline is the page’s, worded from the failure’s code in the reader’s language, and, when the vendor names it, the moment it works again. What the vendor said stays as it came, untranslated, under Provider Response, which folds open under a chevron. The status, the code and the request ids stay in the copied report. Copy takes the whole report. A turn’s failure carries no button: Resume sits in the dock above the composer."
    >
      <GallerySpecimen wide variant="A usage limit · the headline from the code with when it lifts · the vendor payload behind a disclosure">
        <ErrorNotice
          :label="failureHeadline('usage_limit', lifts, now)"
          :raw="usageLimitPayload"
          copy-text="The usage limit has been reached"
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="A rate limit · the headline from the code, the vendor’s sentence behind the disclosure">
        <ErrorNotice
          :label="failureHeadline('rate_limit', null, now)"
          :raw="rateResponse"
          copy-text="Anthropic API request failed with HTTP 429"
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="A vendor’s own code · the neutral line, the vendor’s text behind the disclosure">
        <ErrorNotice
          :label="failureHeadline('CreditsError', null, now)"
          :raw="creditsResponse"
          copy-text="OpenAI API request failed with HTTP 401"
        />
      </GallerySpecimen>
      <GallerySpecimen wide variant="A refused frame · the catalogue’s words, the socket’s Request ID under them">
        <ErrorNotice
          :label="refusalFailure('conversation_archived').message"
          :request-id="refusalFailure('conversation_archived').requestId"
        />
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="RegionStatus · Content Cannot Be Shown"
      note="One pane for every region: the session, a settings list, the sidebar, a dialog body. A glyph for the kind, one sentence, the reason, and Retry, which returns the region to loading in the same frame, before the retried work answers; only a new failure brings the reason back. This specimen’s Retry fails again after a moment, as a backend that still fails the read would."
    >
      <div class="grid gap-6 lg:grid-cols-3">
        <GallerySpecimen wide variant="Failed">
          <div class="rounded-xl border border-line bg-surface">
            <!-- Retry loads again and, as the gallery has no backend, fails the same way after a moment. -->
            <RegionStatus
              status="failed"
              label="Couldn’t load this conversation."
              loading-label="Loading conversation…"
              :detail="unreadable.message"
              :request-id="unreadable.requestId"
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
            <RegionStatus status="note" label="No messages" />
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
      <GallerySpecimen wide variant="A refused request · its Request ID under the line, which Copy takes">
        <InlineError :message="refused.message" :request-id="refused.requestId" />
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="Composer Notices · a State, Not a Failure"
      note="SessionNoticeBar replaces the composer input for a state the reader chose or can leave."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Archived">
          <!-- On the reading surface, where the dock shows it. -->
          <div class="gallery-frame bg-surface p-3">
            <SessionNoticeBar
              :label="agentText('session.archived')"
              :action="agentText('session.restore')"
              @action="productWould('Restore the Conversation')"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Toast · the Request Itself Failed"
      note="A save that did not reach the server, a fork or edit the server refused, a revoke, a background refresh: the reader cannot fix these on the page, so the page shows nothing inline. Title, and a message only when it adds a fact. Success is silent."
    >
      <GallerySpecimen wide variant="Danger · the request's ID under the message, which Copy takes">
        <Toast
          v-if="toastShown"
          :title="agentText('composer.couldntAttach')"
          :message="tooLarge.message"
          tone="danger"
          :request-id="tooLarge.requestId"
          @dismiss="toastShown = false"
        />
        <Button v-else size="sm" @click="toastShown = true">Show Again</Button>
      </GallerySpecimen>
      <GallerySpecimen wide variant="The page's own failure · reported, its ID shown once the report is in">
        <Button size="sm" @click="failOnThePage">Read a Malformed Answer</Button>
      </GallerySpecimen>
    </GallerySection>
  </div>
</template>
