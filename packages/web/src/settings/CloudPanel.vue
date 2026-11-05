<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { t } from '@demicodes/web-ui/settings/locales/text'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { reportError, shownError } from '@demicodes/web-ui/infra/errors'
import { showToast } from '@demicodes/web-ui/infra/toast'
import SettingsCloud from '@demicodes/web-ui/settings/SettingsCloud.vue'
import {
  destinationProblemText,
  type CloudIsolationSetting,
  type CloudNetworkSetting,
  type DestinationProblem,
  type Isolation,
  type PrivateNetworkAccess,
} from '@demicodes/web-ui/settings/cloud'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  cloudIsolationSchema,
  cloudNetworkSchema,
  type CloudIsolation,
  type CloudNetwork,
  type SetCloudIsolation,
} from '../api/generated/web-api'

/**
 * Settings › Cloud for the product. Only the master has the page, so its
 * settings are read when the page opens rather than kept in the product
 * state. A change is stored even when the machine manager cannot take it
 * (`cloud_unavailable`): the page shows it as saved and says when it applies.
 */
const isolation = ref<CloudIsolationSetting | null>(null)
const network = ref<CloudNetworkSetting | null>(null)
const load = ref<'loading' | 'ready' | 'failed'>('loading')
const loadDetail = ref<string | null>(null)

/** The isolation as the page shows it. */
function shown(read: CloudIsolation): CloudIsolationSetting {
  return {
    isolation: read.isolation,
    runtimes:
      read.runtimes?.map((runtime) => ({
        isolation: runtime.isolation,
        unavailable: runtime.unavailable ?? null,
        privateNetwork: runtime.privateNetwork,
      })) ?? null,
  }
}

async function read(): Promise<void> {
  load.value = 'loading'
  try {
    const [isolationRead, networkRead] = await Promise.all([
      apiRequest('/settings/cloud-isolation').then((response) => readResponse(response, cloudIsolationSchema)),
      apiRequest('/settings/cloud-network').then((response) => readResponse(response, cloudNetworkSchema)),
    ])
    isolation.value = shown(isolationRead)
    network.value = networkRead
    load.value = 'ready'
  } catch (error) {
    loadDetail.value = shownError(error).message
    load.value = 'failed'
    reportError(t('cloud.loadFailed'), error, { site: 'cloud.loadFailed' })
  }
}

onMounted(read)

/** Chooses `next`; Clouds move as they stop, which a toast says. */
async function setIsolation(next: Isolation): Promise<void> {
  const moving = { title: t('cloud.title'), message: t('cloud.moving', { isolation: t(`cloud.isolations.${next}`) }), tone: 'neutral' } as const
  try {
    const request: SetCloudIsolation = { isolation: next }
    const response = await apiRequest('/settings/cloud-isolation', { method: 'PUT', ...jsonBody(request) })
    isolation.value = shown(await readResponse(response, cloudIsolationSchema))
    showToast(moving)
  } catch (error) {
    if (error instanceof ApiError && error.code === 'cloud_unavailable' && isolation.value) {
      isolation.value = { ...isolation.value, isolation: next }
      showToast({ title: t('cloud.title'), message: t('cloud.pending'), tone: 'neutral' })
      return
    }
    reportError(t('cloud.isolationFailed'), error, { site: 'cloud.isolationFailed', userVisible: true })
  }
}

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
      showToast({ title: t('cloud.title'), message: t('cloud.pending'), tone: 'neutral' })
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
    showToast({ title: t('cloud.failed'), message: refusal, tone: 'danger' })
  }
}

function saveDestinations(destinations: string[]): Promise<SentenceText | null> {
  return put({ access: 'only', destinations })
}
</script>

<template>
  <SettingsCloud
    :isolation="isolation"
    :network="network"
    :load="load"
    :load-detail="loadDetail"
    :on-retry="read"
    :overlay-store="appOverlayStore"
    :on-set-isolation="setIsolation"
    :on-set-access="setAccess"
    :save-destinations="saveDestinations"
  />
</template>
