<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
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
  type IsolationDownload,
  type PrivateNetworkAccess,
} from '@demicodes/web-ui/settings/cloud'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  cloudIsolationSchema,
  cloudNetworkSchema,
  type CloudIsolation,
  type CloudNetwork,
  type IsolationDownload as IsolationDownloadRead,
  type SetCloudIsolation,
} from '../api/generated/web-api'
import { useHeldList, useProduct } from '../state/product'

/**
 * Settings › Cloud for the product. Only the master has the page. The
 * isolation is a list the page holds while it shows (`cloud_isolation`),
 * which the channel changes as the master chooses and as the server
 * downloads gVisor; Private Network Access is read when the page opens. A
 * change is stored even when the machine manager cannot take it
 * (`cloud_unavailable`): the page shows it as saved and says when it applies.
 */
const product = useProduct()
useHeldList('cloud_isolation')
const network = ref<CloudNetworkSetting | null>(null)
const networkLoad = ref<'loading' | 'ready' | 'failed'>('loading')
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
        download: runtime.download ? downloadShown(runtime.download) : null,
      })) ?? null,
  }
}

/** A download as the page shows it, whose total is null when the source doesn't say it. */
function downloadShown(download: IsolationDownloadRead): IsolationDownload {
  return download.state === 'downloading' ? { ...download, total: download.total ?? null } : download
}

const isolation = computed(() => {
  const read = product.snapshot?.cloudIsolation
  return read ? shown(read) : null
})

const load = computed(() => {
  const isolationLoad = product.listLoad('cloud_isolation')
  if (isolationLoad === 'failed' || networkLoad.value === 'failed') {
    return 'failed'
  }
  return isolationLoad === 'ready' && networkLoad.value === 'ready' ? 'ready' : 'loading'
})

async function readNetwork(): Promise<void> {
  networkLoad.value = 'loading'
  try {
    network.value = await readResponse(await apiRequest('/settings/cloud-network'), cloudNetworkSchema)
    networkLoad.value = 'ready'
  } catch (error) {
    loadDetail.value = shownError(error).message
    networkLoad.value = 'failed'
    reportError(t('cloud.loadFailed'), error, { site: 'cloud.loadFailed' })
  }
}

onMounted(readNetwork)

function retry(): void {
  if (product.listLoad('cloud_isolation') === 'failed') {
    product.retryList('cloud_isolation')
  }
  if (networkLoad.value === 'failed') {
    void readNetwork()
  }
}

/**
 * Chooses `next`; Clouds move as they stop, which a toast says, after the
 * server downloads `next` when it must. Leaving an isolation that awaits its
 * download moves nothing, since no Cloud runs on it, so no toast says so.
 */
async function setIsolation(next: Isolation): Promise<void> {
  const runtimes = isolation.value?.runtimes
  const downloads = runtimes?.find((runtime) => runtime.isolation === next)?.download
  const left = runtimes?.find((runtime) => runtime.isolation === isolation.value?.isolation)?.download
  const message = left
    ? null
    : downloads && downloads.state !== 'starting'
      ? t('cloud.download.moving')
      : t('cloud.moving', { isolation: t(`cloud.isolations.${next}`) })
  const at = product.sent()
  try {
    const request: SetCloudIsolation = { isolation: next }
    const response = await apiRequest('/settings/cloud-isolation', { method: 'PUT', ...jsonBody(request) })
    product.answered(at, { type: 'cloud_isolation', isolation: await readResponse(response, cloudIsolationSchema) })
    if (message) {
      showToast({ title: t('cloud.title'), message, tone: 'neutral' })
    }
  } catch (error) {
    // The channel brings the choice the server saved.
    if (error instanceof ApiError && error.code === 'cloud_unavailable') {
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
    :on-retry="retry"
    :overlay-store="appOverlayStore"
    :on-set-isolation="setIsolation"
    :on-set-access="setAccess"
    :save-destinations="saveDestinations"
  />
</template>
