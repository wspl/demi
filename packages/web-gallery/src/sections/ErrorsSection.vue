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
    where: 'In the conversation: a failed turn, an undelivered message, a lost connection',
    component: 'ErrorNotice',
    form: 'A tinted bar in the transcript flow: sentence, upstream message, facts, Copy, and Retry where the failure is the tail. Never under the composer.',
    examples: 'Provider errors, undelivered message, reconnect failed, refused request',
  },
  {
    where: 'The content of a region cannot be shown',
    component: 'RegionStatus',
    form: 'Replaces the region: one sentence, the reason, Retry. Retry returns to loading.',
    examples: 'Session restore, conversation list, settings lists, directory read',
  },
  {
    where: 'A form rejected its own input',
    component: 'InlineError',
    form: 'A line of text under the fields, in their width. The form\'s submit is the retry.',
    examples: 'Sign in, codes, passwords, project creation, folder creation',
  },
  {
    where: 'The request itself failed, not the input',
    component: 'Toast',
    form: 'Through reportError: title, and a message only when it adds a fact. Never inline, never for success.',
    examples: 'Save failed, fork failed, edit refused, unreadable attachment, sign out, revoke device',
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
