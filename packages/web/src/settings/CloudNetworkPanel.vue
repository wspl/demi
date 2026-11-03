<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { t } from '@demicodes/web-ui/settings/locales/text'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { reportError, shownError } from '@demicodes/web-ui/infra/errors'
import { showToast } from '@demicodes/web-ui/infra/toast'
import SettingsCloudNetwork from '@demicodes/web-ui/settings/SettingsCloudNetwork.vue'
import {
  destinationProblemText,
  type CloudNetworkSetting,
  type DestinationProblem,
  type PrivateNetworkAccess,
} from '@demicodes/web-ui/settings/cloud-network'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import { cloudNetworkSchema, type CloudNetwork } from '../api/generated/web-api'

/**
 * Settings › Cloud Network for the product. Only the master has the page, so
 * the setting is read when the page opens rather than kept in the product
 * state. A change is stored even when the machine manager cannot take it
 * (`cloud_unavailable`): the page shows it as saved and says when it applies.
 */
const network = ref<CloudNetworkSetting | null>(null)
const load = ref<'loading' | 'ready' | 'failed'>('loading')
const loadDetail = ref<string | null>(null)

async function read(): Promise<void> {
  load.value = 'loading'
  try {
    network.value = await readResponse(await apiRequest('/settings/cloud-network'), cloudNetworkSchema)
    load.value = 'ready'
  } catch (error) {
    loadDetail.value = shownError(error).message
    load.value = 'failed'
    reportError(t('cloudNetwork.loadFailed'), error, { site: 'cloudNetwork.loadFailed' })
  }
}

onMounted(read)

/** Stores `next`, answering why the server refused it, or null once stored. */
async function put(next: CloudNetworkSetting): Promise<SentenceText | null> {
  try {
    const request: CloudNetwork = { access: next.access, destinations: next.destinations }
    const response = await apiRequest('/settings/cloud-network', { method: 'PUT', ...jsonBody(request) })
    network.value = await readResponse(response, cloudNetworkSchema)
    return null
  } catch (error) {
    if (error instanceof ApiError && error.code === 'cloud_unavailable') {
      network.value = next
      showToast({ title: t('cloudNetwork.title'), message: t('cloudNetwork.pending'), tone: 'neutral' })
      return null
    }
    if (error instanceof ApiError && error.code === 'invalid_body' && typeof error.params.problem === 'string') {
      return destinationProblemText(error.params.problem as DestinationProblem)
    }
    return shownError(error).message
  }
}

async function setAccess(access: PrivateNetworkAccess): Promise<void> {
  if (!network.value) {
    return
  }
  const refusal = await put({ ...network.value, access })
  if (refusal !== null) {
    showToast({ title: t('cloudNetwork.failed'), message: refusal, tone: 'danger' })
  }
}

function saveDestinations(destinations: string[]): Promise<SentenceText | null> {
  return put({ access: 'only', destinations })
}
</script>

<template>
  <SettingsCloudNetwork
    :network="network"
    :load="load"
    :load-detail="loadDetail"
    :on-retry="read"
    :overlay-store="appOverlayStore"
    :on-set-access="setAccess"
    :save-destinations="saveDestinations"
  />
</template>
