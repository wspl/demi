<script setup lang="ts">
import { onBeforeUnmount, reactive, ref } from 'vue'
import AgentMessageVirtualBlock from '@demicodes/web-ui/agent/blocks/AgentMessageVirtualBlock.vue'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import UndeliveredRetryChip from '@demicodes/web-ui/agent/UndeliveredRetryChip.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { applyAttachmentUpdate, composerAttachment } from '@demicodes/web-ui/agent/message-input/attachments'
import { joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import type { Block, UserContentBlock } from '@demicodes/protocol'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { useTurnFlow } from '../turn-flow'
import { provideCredentialKinds } from '@demicodes/web-ui/agent/credential-kinds'
import { demoModel, errorTool } from '../fixtures/blocks'
import { galleryBlobs, galleryBlobUrl } from '../fixtures/blobs'
import { sweepUpload } from '../fixtures/upload-sweep'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Failures that live in the conversation, pinned. Every one is an ErrorNotice
 * in the transcript flow, rendered through the product's own block renderer so
 * the padding and rhythm are the product's. None carries a button: Resume
 * and Retry sit in the dock above the composer, which each specimen of an
 * undelivered message shows under its list.
 *
 * Nothing in or under the composer: a fork that failed, an edit the server
 * refused and a file that could not be read are toasts, and a file whose
 * upload failed keeps its capsule with Retry. A sent message whose upload
 * failed is undelivered, in the transcript flow, where it was sent.
 *
 * Each record is named by its normalized code, which is also its id, and
 * runs on a subscription; a record given `onKey`, its name, ran on an API
 * key entry, whose `auth_expired` names its key, never a sign-in. Each
 * carries the `http` record a provider stores: the status, the response's
 * headers, and the body, the vendor's part of `message`.
 */
function errorRecord(
  code: string,
  httpStatus: number,
  message: string,
  onKey?: string,
): Block {
  return {
    type: 'error',
    id: onKey ?? code,
    createdAt: '2026-09-11T12:00:00Z',
    model: onKey === undefined ? demoModel : { ...demoModel, providerId: KEY_ENTRY },
    message,
    code,
    diagnostics: {
      source: 'http',
      httpStatus,
      clientRequestId: 'req_01J8Y3Q6ZKX4',
      upstream: JSON.stringify({
        status: httpStatus,
        headers: [
          ['cf-ray', 'a4c1c9a578b78e6c-PDX'],
          ['content-type', 'application/json'],
          ['date', 'Fri, 11 Sep 2026 12:00:00 GMT'],
          ['server', 'cloudflare'],
        ],
        body: /HTTP \d+: (.*)$/s.exec(message)?.[1] ?? message,
      }),
    },
  }
}
/** The Compatible API entry the key records ran on; every other entry is a subscription. */
const KEY_ENTRY = 'compatible-api'
provideCredentialKinds(() => (providerId) => providerId === KEY_ENTRY ? 'api_key' : 'subscription')
const records: Block[] = [
  errorRecord('usage_limit', 429, 'The usage limit has been reached'),
  errorRecord(
    'rate_limit',
    429,
    'Anthropic API request failed with HTTP 429: This request would exceed the rate limit of 50 requests per minute for your organization. Retry after 12 seconds.',
  ),
  errorRecord('overloaded', 529, 'Overloaded. The provider could not accept the request.'),
  errorRecord('auth_expired', 401, 'The access token has expired. Sign in to the provider again.'),
  errorRecord(
    'auth_expired',
    401,
    'OpenAI API request failed with HTTP 401: {"error":{"message":"Incorrect API key provided: sk-…a1b2.","type":"invalid_request_error","code":"invalid_api_key"}}',
    'auth_expired · API key',
  ),
  errorRecord(
    'permission_denied',
    403,
    'OpenAI API request failed with HTTP 403: {"error":{"message":"Your Go plan doesn\'t include API access. Upgrade to Provider or higher at https://commandcode.ai/billing to use these endpoints.","type":"permission_error","code":"upgrade_required"}}',
    'permission_denied · API key',
  ),
  errorRecord('auth_missing', 401, 'No credentials are configured for this provider.'),
  errorRecord(
    'context_length_exceeded',
    400,
    'prompt is too long: 214,331 tokens > 200,000 maximum.',
  ),
  errorRecord(
    'request_too_large',
    413,
    'Anthropic API request failed with HTTP 413: Request exceeds the maximum allowed number of bytes.',
  ),
  errorRecord(
    'generation_failed',
    502,
    'The upstream response ended before generation completed.',
  ),
]
/** The failed call; Run Again runs it and fails it while it is on screen, where it stays folded too. */
const toolFailure = ref<ToolCallBlock>(errorTool)
const TOOL_RUN_MS = 1200
let toolRunTimer: ReturnType<typeof setTimeout> | undefined

function runToolAgain(): void {
  clearTimeout(toolRunTimer)
  toolFailure.value = { ...errorTool, status: 'executing', output: [] }
  toolRunTimer = setTimeout(() => {
    toolRunTimer = undefined
    toolFailure.value = errorTool
  }, TOOL_RUN_MS)
}

onBeforeUnmount(() => clearTimeout(toolRunTimer))
// The exact message stays where it was sent, the failure under it, and the
// message sent after it comes below with its turn. Retry sends it again
// with the same id, as a new message at the end, and Requesting shows until
// its turn answers.
const undeliveredFlow = useTurnFlow({ id: 'gallery-undelivered-message' })
const undeliveredText = `Please keep this exact message, the plan ${ATTACHMENT_MARK} and the capture ${ATTACHMENT_MARK}.`
const undeliveredFiles = [
  { name: 'plan.pdf', mediaType: 'application/pdf' },
  { name: 'reference.png', mediaType: 'image/png' },
]

function failDelivery(): void {
  undeliveredFlow.undelivered(
    joinMessageContent(undeliveredText, undeliveredFiles.map(({ name, mediaType }, index) => [{
      type: 'attachment',
      name,
      id: `a${index + 1}`,
      mediaType,
      sizeBytes: 4096,
      sha256: '0'.repeat(64),
    } satisfies UserContentBlock])),
    {
      text: undeliveredText,
      attachments: undeliveredFiles.map(({ name }) => composerAttachment({ name, phase: 'ready' })),
    },
    // A failure inside the backend, which has no cause of its own, names
    // the request to quote.
    'The message couldn’t be sent.',
    'req_ffb05w4w6kxpdtz0',
  )
  sendAnother()
}

/** A message sent after the one not delivered, which comes below it with its turn. */
function sendAnother(): void {
  const text = 'Meanwhile, list the open issues.'
  undeliveredFlow.turn([{ type: 'text', text }], { text, attachments: [] })
}
failDelivery()

// A message sent while its file uploads shows at once, the capsule telling
// the upload, and goes once the upload is done; one whose upload fails is
// undelivered, and Retry uploads the file again, then delivers the message.
const captureText = `What goes wrong in this capture ${ATTACHMENT_MARK}?`
const captureContent = joinMessageContent(captureText, [[
  { type: 'image', source: { type: 'ref', ref: galleryBlobs.screenshot, mediaType: 'image/png' } },
  {
    type: 'attachment',
    name: 'capture.png',
    id: 'a3',
    mediaType: 'image/png',
    sizeBytes: 48_213,
    sha256: galleryBlobs.screenshot,
  },
] satisfies UserContentBlock[]])
/** The sweeps of the specimens' uploads, which stop with the page. */
const sweeps = new AbortController()
onBeforeUnmount(() => sweeps.abort())

/** The capture's upload sweeping to done, as a slow network uploads it; answers the message's content then. */
function uploadCapture(capture: ReturnType<typeof composerAttachment>): Promise<UserContentBlock[]> {
  return sweepUpload(sweeps.signal, (progress) => applyAttachmentUpdate(capture, { phase: 'uploading', progress }))
    .then(() => {
      applyAttachmentUpdate(capture, { phase: 'ready' })
      return captureContent
    })
}

function newCapture() {
  return reactive(composerAttachment({
    name: 'capture.png',
    src: galleryBlobUrl(galleryBlobs.screenshot, 'image/png'),
    phase: 'uploading',
    progress: 0,
  }))
}

const uploadingFlow = useTurnFlow({ id: 'gallery-sent-while-uploading' })
function sendWhileUploading(): void {
  const capture = newCapture()
  uploadingFlow.turn(captureContent, { text: captureText, attachments: [capture], ready: uploadCapture(capture) })
}
sendWhileUploading()

const uploadFailedFlow = useTurnFlow({ id: 'gallery-upload-failed' })
let failedCapture = newCapture()
function failUpload(): void {
  failedCapture = newCapture()
  applyAttachmentUpdate(failedCapture, { phase: 'failed' })
  uploadFailedFlow.undelivered(
    captureContent,
    { text: captureText, attachments: [failedCapture] },
    'capture.png did not upload. The server could not store it.',
  )
}
failUpload()

// A file of the message is gone, its bytes with the page that had them:
// sending again cannot deliver it, so the dock offers no Retry; its Edit
// takes the message back to the composer, where the file shows as missing.
const goneFlow = useTurnFlow({ id: 'gallery-file-gone' })
function loseFile(): void {
  goneFlow.undelivered(
    captureContent,
    { text: captureText, attachments: [composerAttachment({ name: 'capture.png', phase: 'missing' })] },
    'A file of this message is no longer available. Attach it again.',
    null,
    true,
  )
}
loseFile()
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Turn Failures · the Record in the Transcript"
      note="The page’s headline for the failure’s code, in the reader’s language; what the provider said, untranslated, under Provider Response; the facts a support thread asks for in Copy. No record carries a button: Resume sits in the dock above the composer."
    >
      <GallerySpecimen
        v-for="block in records"
        :key="block.id"
        wide
        :variant="block.id"
      >
        <div class="rounded-lg bg-surface py-3">
          <AgentMessageVirtualBlock
            :block="block"
            :is-thinking-streaming="false"
            :thinking-ended-at="null"
          />
        </div>
      </GallerySpecimen>
      <GallerySpecimen wide variant="Tool failed · the tool block is the record, folded until the user opens it">
        <div class="rounded-lg bg-surface py-3">
          <AgentMessageVirtualBlock
            :block="toolFailure"
            :is-thinking-streaming="false"
            :thinking-ended-at="null"
          />
        </div>
        <Button size="sm" variant="ghost" class="mt-2" @click="runToolAgain">Run Again</Button>
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="Undelivered Message · the Notice Follows the Message"
      note="The exact text and its attachments stay where they were sent, a message sent later and its turn below them; the failure follows in flow, saying why, with no button. The message’s hover actions offer Copy, Edit, which takes the message back to the composer, and Delete, which takes it away at once. Retry sits in the dock, shown here under each list: it sends every undelivered message again in the order they were sent, each as a new message at the end. A refusal sending again cannot change, such as a file that is gone, is skipped, and with no other left there is no Retry."
    >
      <GallerySpecimen wide variant="Send failed · a later message below it">
        <div class="flex h-[26rem] flex-col overflow-hidden rounded-lg bg-surface phone:[--agent-pad-x:0.75rem]">
          <AgentMessageList
            class="min-h-0 flex-1"
            :conversation-id="undeliveredFlow.state.id"
            :blocks="undeliveredFlow.state.blocks"
            :pending-steers="[]"
            :queue="[]"
            :phase="undeliveredFlow.state.phase"
            :load="undeliveredFlow.state.load"
            :pending-submissions="undeliveredFlow.pendingSubmissions.value"
            :removing="undeliveredFlow.removing.value"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
            @edit-submission="undeliveredFlow.editSubmission"
            @delete-submission="undeliveredFlow.deleteSubmission"
          />
        </div>
        <div class="mt-2 flex items-center gap-2">
          <UndeliveredRetryChip :pending-submissions="undeliveredFlow.pendingSubmissions.value" @retry="undeliveredFlow.retrySubmissions()" />
          <Button size="sm" @click="sendAnother">Send Another</Button>
          <Button size="sm" @click="failDelivery">Fail Again</Button>
        </div>
      </GallerySpecimen>
      <GallerySpecimen wide variant="Sent while its file uploads">
        <div class="flex h-[14rem] flex-col overflow-hidden rounded-lg bg-surface phone:[--agent-pad-x:0.75rem]">
          <AgentMessageList
            class="min-h-0 flex-1"
            :conversation-id="uploadingFlow.state.id"
            :blocks="uploadingFlow.state.blocks"
            :pending-steers="[]"
            :queue="[]"
            :phase="uploadingFlow.state.phase"
            :load="uploadingFlow.state.load"
            :pending-submissions="uploadingFlow.pendingSubmissions.value"
            :removing="uploadingFlow.removing.value"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
            @edit-submission="uploadingFlow.editSubmission"
            @delete-submission="uploadingFlow.deleteSubmission"
          />
        </div>
        <div class="mt-2 flex items-center gap-2">
          <UndeliveredRetryChip :pending-submissions="uploadingFlow.pendingSubmissions.value" @retry="uploadingFlow.retrySubmissions()" />
          <Button size="sm" @click="sendWhileUploading">Send Again</Button>
        </div>
      </GallerySpecimen>
      <GallerySpecimen wide variant="Upload failed">
        <div class="flex h-[16rem] flex-col overflow-hidden rounded-lg bg-surface phone:[--agent-pad-x:0.75rem]">
          <AgentMessageList
            class="min-h-0 flex-1"
            :conversation-id="uploadFailedFlow.state.id"
            :blocks="uploadFailedFlow.state.blocks"
            :pending-steers="[]"
            :queue="[]"
            :phase="uploadFailedFlow.state.phase"
            :load="uploadFailedFlow.state.load"
            :pending-submissions="uploadFailedFlow.pendingSubmissions.value"
            :removing="uploadFailedFlow.removing.value"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
            @edit-submission="uploadFailedFlow.editSubmission"
            @delete-submission="uploadFailedFlow.deleteSubmission"
          />
        </div>
        <div class="mt-2 flex items-center gap-2">
          <UndeliveredRetryChip :pending-submissions="uploadFailedFlow.pendingSubmissions.value" @retry="uploadFailedFlow.retrySubmissions(() => uploadCapture(failedCapture))" />
          <Button size="sm" @click="failUpload">Fail Again</Button>
        </div>
      </GallerySpecimen>
      <GallerySpecimen wide variant="A file is gone · Edit, and no Retry">
        <div class="flex h-[14rem] flex-col overflow-hidden rounded-lg bg-surface phone:[--agent-pad-x:0.75rem]">
          <AgentMessageList
            class="min-h-0 flex-1"
            :conversation-id="goneFlow.state.id"
            :blocks="goneFlow.state.blocks"
            :pending-steers="[]"
            :queue="[]"
            :phase="goneFlow.state.phase"
            :load="goneFlow.state.load"
            :pending-submissions="goneFlow.pendingSubmissions.value"
            :removing="goneFlow.removing.value"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
            @edit-submission="goneFlow.editSubmission"
            @delete-submission="goneFlow.deleteSubmission"
          />
        </div>
        <div class="mt-2 flex items-center gap-2">
          <UndeliveredRetryChip :pending-submissions="goneFlow.pendingSubmissions.value" @retry="goneFlow.retrySubmissions()" />
          <Button size="sm" @click="loseFile">Fail Again</Button>
        </div>
      </GallerySpecimen>
    </GallerySection>
  </div>
</template>
