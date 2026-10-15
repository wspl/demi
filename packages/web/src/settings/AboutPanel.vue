<script setup lang="ts">
import { t } from './locales/text'
import { computed, onUnmounted, ref, watch } from 'vue'
import SettingsAbout from '@demicodes/web-ui/settings/SettingsAbout.vue'
import type { AcknowledgementsState } from '@demicodes/web-ui/settings/types'
import { shownError } from '@demicodes/web-ui/infra/errors'
import { useSettingsAddress } from './address'

/**
 * Settings › About for the product: the version this page's build names,
 * and the release's third-party notices, which the built web app holds as
 * `third-party-notices.txt` (`xtask notices`), read when Acknowledgements
 * opens. Acknowledgements is `/settings/about/acknowledgements`, a step of
 * the history as a device's page is.
 */
const ACKNOWLEDGEMENTS = 'acknowledgements'
/** The version this page's build names (`vite.config.ts`). */
const version = import.meta.env.DEMI_VERSION

const address = useSettingsAddress()
const open = computed(() => address.section.value === 'about' && address.detail.value === ACKNOWLEDGEMENTS)

const acknowledgements = ref<AcknowledgementsState>({ status: 'loading' })
let request: AbortController | null = null

async function load(): Promise<void> {
  request?.abort()
  const controller = new AbortController()
  request = controller
  acknowledgements.value = { status: 'loading' }
  try {
    // Plain text only: Vite's development server answers a request that
    // takes any type with the page.
    const response = await fetch('/third-party-notices.txt', {
      headers: { accept: 'text/plain' },
      signal: controller.signal,
    })
    if (response.status === 404) {
      acknowledgements.value = { status: 'missing' }
      return
    }
    if (!response.ok) {
      throw new Error(`The server answered ${response.status}.`)
    }
    acknowledgements.value = { status: 'ready', text: await response.text() }
  } catch (error) {
    if (!controller.signal.aborted) {
      acknowledgements.value = { status: 'failed', error: shownError(error, t('about.acknowledgementsFailed')) }
    }
  } finally {
    if (request === controller) {
      request = null
    }
  }
}

// Read once, when the page first opens; a failure is read again on Retry.
watch(open, (isOpen) => {
  if (isOpen && acknowledgements.value.status === 'loading' && request === null) {
    void load()
  }
}, { immediate: true })
onUnmounted(() => request?.abort())
</script>

<template>
  <SettingsAbout
    :version="version"
    :acknowledgements="acknowledgements"
    :acknowledgements-open="open"
    @update:acknowledgements-open="address.show('about', $event ? ACKNOWLEDGEMENTS : null)"
    @retry="load"
  />
</template>
