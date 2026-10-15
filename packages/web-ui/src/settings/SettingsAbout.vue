<script setup lang="ts">
import AsyncRegion from '../ui/AsyncRegion.vue'
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { AcknowledgementsState } from './types'

/**
 * Settings › About: the Demi version, and Acknowledgements, a page of its
 * own with a back button to About, as a row of System Settings opens one.
 * Acknowledgements shows the third-party notices of the release, the
 * licenses of the open-source software it includes
 * (`builds-and-releases.md` § Licenses in a release).
 */
defineProps<{
  version: string
  acknowledgements: AcknowledgementsState
}>()

/** Whether the Acknowledgements page is open rather than About. */
const acknowledgementsOpen = defineModel<boolean>('acknowledgementsOpen', { default: false })

const emit = defineEmits<{
  /** Load the notices again after a failure. */
  retry: []
}>()
</script>

<template>
  <SettingsPage
    v-if="acknowledgementsOpen"
    :title="t('about.acknowledgements.title')"
    :description="acknowledgements.status === 'missing' ? undefined : t('about.acknowledgements.description')"
    :back="t('about.title')"
    @back="acknowledgementsOpen = false"
  >
    <SettingsGroup>
      <AsyncRegion
        :state="acknowledgements.status === 'failed' ? 'failed' : acknowledgements.status === 'loading' ? 'loading' : 'ready'"
        :label="t('about.acknowledgements.loading')"
        :error="acknowledgements.status === 'failed' ? acknowledgements.error.message : undefined"
        :request-id="acknowledgements.status === 'failed' ? acknowledgements.error.requestId : null"
        @retry="emit('retry')"
      >
        <pre
          v-if="acknowledgements.status === 'ready'"
          class="select-text overflow-x-auto whitespace-pre-wrap break-words px-4 py-3 font-mono text-[12px] leading-[18px] text-fg-body"
        >{{ acknowledgements.text }}</pre>
        <div
          v-else
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          {{ t('about.acknowledgements.none') }}
        </div>
      </AsyncRegion>
    </SettingsGroup>
  </SettingsPage>
  <SettingsPage v-else :title="t('about.title')">
    <SettingsGroup>
      <SettingsRow :label="t('about.version')">
        <span class="select-text text-[13px] tabular-nums text-fg-body">{{ version }}</span>
      </SettingsRow>
      <SettingsRow
        :label="t('about.acknowledgements.title')"
        :description="t('about.acknowledgements.description')"
        navigable
        @click="acknowledgementsOpen = true"
      />
    </SettingsGroup>
  </SettingsPage>
</template>
