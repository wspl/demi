<script setup lang="ts">
import Toast from '@demicodes/web-ui/ui/Toast.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import RegionStatus from '@demicodes/web-ui/ui/RegionStatus.vue'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import SessionNoticeBar from '@demicodes/web-ui/agent/SessionNoticeBar.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * The four error primitives, each pinned in every variant it has, and the
 * product's toasts. This is the reference for the rest of the Errors page:
 * a failure is shown by exactly one of these, chosen by where it belongs.
 */
const regions = [
  { label: 'Devices', error: "Couldn't load devices." },
  { label: 'Providers', error: "Couldn't load providers." },
  { label: 'Vendor catalog', error: "Couldn't load the vendor catalog." },
  { label: 'Projects', error: "Couldn't load projects." },
]
/** Product toasts as the product raises them: a title, and a message only when it adds a fact. */
const toasts = [
  { origin: 'App · signOut', title: 'Could not sign out', message: 'Failed to fetch' },
  { origin: 'settings/devices · revoke', title: 'Could not revoke device', message: 'HTTP 409: the device is still paired.' },
  { origin: 'state/preferences · save', title: 'Could not update settings', message: 'Failed to fetch' },
  { origin: 'settings/providers · any request', title: 'Provider operation failed', message: 'Your endpoint changes could not be saved.' },
  { origin: 'conversation store · notice', title: 'This conversation is still running.' },
  { origin: 'composer · rejected drop', title: 'Unsupported attachment type', message: 'archive.tar.gz, notes.rtf' },
  { origin: 'composer · removed failed upload', title: "Couldn't attach", message: 'reference.png' },
]
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="RegionStatus · content cannot be shown"
      note="One pane for every region: the session, a settings list, the sidebar, a dialog body. Failed regions carry the reason and Retry; loading and empty regions only a sentence."
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
        <GallerySpecimen wide variant="Failed · retry pending">
          <div class="rounded-xl border border-line bg-surface">
            <RegionStatus
              failed
              label="Couldn't load this conversation."
              action="Retry"
              action-pending
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
        <GallerySpecimen wide variant="Model selector · catalog failed">
          <div class="rounded-xl border border-line bg-surface p-3">
            <ModelSelector load="failed" :providers="[]" :models="{}" />
          </div>
        </GallerySpecimen>
      </div>
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen
          v-for="region in regions"
          :key="region.label"
          wide
          :variant="`AsyncRegion · ${region.label}`"
        >
          <div class="rounded-xl border border-line bg-surface">
            <AsyncRegion state="failed" :error="region.error">
              <p class="p-6 text-chrome text-fg-muted">{{ region.label }} loaded.</p>
            </AsyncRegion>
          </div>
        </GallerySpecimen>
      </div>
      <GallerySpecimen wide variant="Archived list · failed initial read">
        <div class="rounded-xl border border-line bg-surface">
          <SettingsArchived load="failed" :conversations="[]" />
        </div>
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="InlineError · an action failed, the input stays"
      note="Under the control that failed, in its width. One optional action, one optional dismiss. Long text wraps; several reasons stack as lines."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Message only">
          <InlineError message="The email or password is incorrect." />
        </GallerySpecimen>
        <GallerySpecimen wide variant="With action">
          <InlineError
            message="Could not save the endpoint. Your changes are retained."
            action="Retry save"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Action pending">
          <InlineError
            message="Could not save the endpoint. Your changes are retained."
            action="Retry save"
            action-pending
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Dismissible">
          <InlineError
            message="This value cannot be saved because the service is unavailable. Keep the value and try again after reconnecting."
            dismissible
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Several reasons · one per line">
          <InlineError
            :message="'reference.png: Connection lost during upload.\nplan.pdf: The workspace is unavailable.'"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Long text · wraps beside the action">
          <InlineError
            message="The upstream response ended before generation completed. Request preview-7842 returned HTTP 502. The draft and the preceding conversation remain available, and a retry sends the same content again."
            action="Retry"
          />
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="SessionNoticeBar · a condition of the whole session"
      note="Neutral for a state the reader chose or can leave; danger for a failure that lasts until the session recovers. One action at most. It never repeats a pane or a record."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Danger · with Retry">
          <SessionNoticeBar
            tone="danger"
            label="Agent socket failed to connect: ECONNREFUSED 127.0.0.1:18911"
            action="Retry"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Danger · no action">
          <SessionNoticeBar
            tone="danger"
            label="The request was refused: another view holds this conversation."
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Neutral · archived">
          <SessionNoticeBar
            label="This conversation is archived."
            action="Restore conversation"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Neutral · no models">
          <SessionNoticeBar
            label="No models available."
            action="Configure models"
          />
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Toast · an outcome with no other surface"
      note="Only for a failure whose control is already gone: the menu closed, the drop had no field, the background save had no page. Success is silent. Each toast is shown as the product raises it."
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
