<script setup lang="ts">
import { t } from './locales/text'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { reportError } from '@demicodes/web-ui/infra/errors'
import SettingsUpdates from '@demicodes/web-ui/settings/SettingsUpdates.vue'
import type { UpgradeChannel } from '@demicodes/web-ui/settings/server-upgrade'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  serverUpgradeSchema,
  workingAgentsSchema,
  type RunUpgrade,
  type SetUpgradeChannel,
} from '../api/generated/web-api'
import { useProduct } from '../state/product'

/**
 * Settings › Updates for the product: the state's `server_upgrade`
 * part, which the channel sends to the master and admins as it changes. Each
 * write answers the part as it left it.
 */
const product = useProduct()

async function write(path: string, init: RequestInit, title: HeadlineText, site: string): Promise<void> {
  const at = product.sent()
  try {
    const upgrade = await readResponse(await apiRequest(path, init), serverUpgradeSchema)
    product.answered(at, { type: 'server_upgrade', upgrade })
  } catch (error) {
    reportError(title, error, { site, userVisible: true })
  }
}

function setChannel(channel: UpgradeChannel): Promise<void> {
  return write(
    '/server-upgrade/channel',
    { method: 'PUT', ...jsonBody({ channel } satisfies SetUpgradeChannel) },
    t('updates.channelFailed'),
    'updates.channelFailed',
  )
}

function check(): Promise<void> {
  return write('/server-upgrade/check', { method: 'POST' }, t('updates.checkFailed'), 'updates.checkFailed')
}

function update(version: string): Promise<void> {
  return write(
    '/server-upgrade/run',
    { method: 'POST', ...jsonBody({ version } satisfies RunUpgrade) },
    t('updates.updateFailed'),
    'updates.updateFailed',
  )
}

async function countAgents(): Promise<number> {
  return (await readResponse(await apiRequest('/server-upgrade/agents'), workingAgentsSchema)).count
}
</script>

<template>
  <SettingsUpdates
    v-if="product.snapshot?.serverUpgrade"
    :state="product.snapshot.serverUpgrade"
    :overlay-store="appOverlayStore"
    :on-set-channel="setChannel"
    :on-check="check"
    :on-count-agents="countAgents"
    :on-update="update"
  />
</template>
