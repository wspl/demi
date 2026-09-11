<script setup lang="ts">
import { useGalleryView } from '../gallery-views'
import AutofocusScope from '@demicodes/web-ui/ui/AutofocusScope.vue'
import GalleryErrorSession from '../components/GalleryErrorSession.vue'
import GalleryErrorMessages from '../components/GalleryErrorMessages.vue'
import GalleryErrorForms from '../components/GalleryErrorForms.vue'
import GalleryErrorFiles from '../components/GalleryErrorFiles.vue'
import GalleryErrorRegions from '../components/GalleryErrorRegions.vue'

const { view } = useGalleryView()

/**
 * How a failure is shown, decided by where it belongs. Every specimen on this
 * page is one of these four; nothing draws its own red text.
 */
const rules = [
  {
    where: 'The content of a region cannot be shown',
    component: 'RegionStatus',
    form: 'Replaces the region: one sentence, the reason, Retry.',
    examples: 'Session restore, conversation list, settings lists, directory read, model catalog',
  },
  {
    where: 'An action failed and its input is still on screen',
    component: 'InlineError',
    form: 'Directly under the control, in its width: message, one action, optional dismiss.',
    examples: 'Sign in, dialogs, saves, send, fork, edit, uploads, folder creation',
  },
  {
    where: 'A condition of the whole session, while it lasts',
    component: 'SessionNoticeBar',
    form: 'A bar in the dock: neutral for a chosen state, danger for a failure; one action.',
    examples: 'Reconnect failed with history, refused request, archived, no models',
  },
  {
    where: 'The turn itself failed',
    component: 'ErrorBlock · tool block',
    form: 'A transcript record with diagnostics and Copy; the dock chip offers Retry.',
    examples: 'Provider errors, tool failures',
  },
  {
    where: 'The control is already gone',
    component: 'Toast',
    form: 'Title, and a message only when it adds a fact. Never for success.',
    examples: 'Sign out, revoke device, background save, rejected drop',
  },
]
</script>

<template>
  <AutofocusScope :enabled="false">
    <div class="space-y-8">
      <section class="space-y-3">
        <p class="max-w-3xl text-[13px] leading-5 text-fg-muted">
          Every failure the product can show, pinned in its failed state. The
          surface is chosen by where the failure belongs, and a failure is named
          exactly once.
        </p>
        <div class="overflow-x-auto rounded-xl border border-line">
          <table class="w-full min-w-[48rem] text-left text-[12px] leading-4">
            <thead class="bg-surface-raised text-fg-subtle">
              <tr>
                <th class="px-3 py-2 font-medium">Where the failure belongs</th>
                <th class="px-3 py-2 font-medium">Surface</th>
                <th class="px-3 py-2 font-medium">Form</th>
                <th class="px-3 py-2 font-medium">Used for</th>
              </tr>
            </thead>
            <tbody class="text-fg-body">
              <tr v-for="rule in rules" :key="rule.component" class="border-t border-line align-top">
                <td class="px-3 py-2">{{ rule.where }}</td>
                <td class="px-3 py-2 font-mono text-fg-emphasis">{{ rule.component }}</td>
                <td class="px-3 py-2">{{ rule.form }}</td>
                <td class="px-3 py-2 text-fg-muted">{{ rule.examples }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
      <GalleryErrorSession v-if="view === 'conversation'" />
      <GalleryErrorMessages v-else-if="view === 'messages'" />
      <GalleryErrorForms v-else-if="view === 'forms'" />
      <GalleryErrorFiles v-else-if="view === 'files'" />
      <GalleryErrorRegions v-else />
    </div>
  </AutofocusScope>
</template>
