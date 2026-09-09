<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { apiRequest, jsonBody } from '../api/client'
import { claimDevice, deviceInstallation } from '../devices/pairing'

const resources = useResources()
const product = useProduct()
const lifetime = new AbortController()
const reset = ref<
  | { status: 'idle' | 'pending' }
  | {
      status: 'failed'
      message: string
    }
>({ status: 'idle' })
const cloud = computed(() =>
  product.snapshot?.cloud
    ? {
        state: product.snapshot.cloud.state,
        phase: product.snapshot.cloud.operation?.phase ?? null,
        error:
          product.snapshot.cloud.error ??
          product.snapshot.cloud.operation?.error ??
          null,
        ...product.snapshot.cloud.limits,
      }
    : null,
)

async function revoke(id: string): Promise<void> {
  try {
    await apiRequest(`/devices/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      signal: lifetime.signal,
    })
    await product.revalidate()
  } catch (error) {
    if (!lifetime.signal.aborted) {
      showToast({
        title: 'Could not revoke device',
        message: error instanceof Error ? error.message : String(error),
        tone: 'danger',
      })
    }
  }
}

async function resetCloud(operationId: string): Promise<void> {
  if (reset.value.status === 'pending') {
    return
  }
  reset.value = { status: 'pending' }
  try {
    await apiRequest('/cloud/reset', {
      method: 'POST',
      signal: lifetime.signal,
      ...jsonBody({ operationId }),
    })
    await product.revalidate()
    reset.value = { status: 'idle' }
  } catch (error) {
    if (!lifetime.signal.aborted) {
      reset.value = {
        status: 'failed',
        message: error instanceof Error ? error.message : String(error),
      }
    }
  }
}

onUnmounted(() => lifetime.abort())
</script>

<template>
  <SettingsDevices
    :devices="resources.devices"
    :cloud="cloud"
    :reset-pending="reset.status === 'pending'"
    :reset-error="reset.status === 'failed' ? reset.message : null"
    @reset-cloud="resetCloud"
    :overlay-store="appOverlayStore"
    :installation="deviceInstallation"
    :claim-device="claimDevice"
    @revoke="revoke"
  />
</template>
