<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import AutomationsPane from '@demicodes/web-ui/automations/AutomationsPane.vue'
import AutomationList from '@demicodes/web-ui/automations/AutomationList.vue'
import AutomationEditor from '@demicodes/web-ui/automations/AutomationEditor.vue'
import { t } from '@demicodes/web-ui/automations/locales/text'
import type { AutomationRun } from '@demicodes/web-ui/automations/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useConversationNavigation } from '../conversation/navigation'
import { openSettings } from '../settings/address'
import { useResources } from '../state/resources'
import { useAutomations } from './store'

/**
 * The Automations page in the main area: the list, or the automation the
 * address names, edited in place; a run opens its conversation.
 */
const props = defineProps<{
  /** The automation the address names; none for the list. */
  id?: string
}>()

const router = useRouter()
const store = useAutomations()
const resources = useResources()
const navigation = useConversationNavigation()

const current = computed(() => (props.id ? store.automations.find((automation) => automation.id === props.id) ?? null : null))
/** The moment the list's last runs are said against, as of its last change. */
const now = computed(() => (store.automations, Date.now()))
/** The user's own time zone first, then every zone the browser knows. */
const timeZones = computed(() => {
  const own = Intl.DateTimeFormat().resolvedOptions().timeZone
  return [own, ...Intl.supportedValuesOf('timeZone').filter((zone) => zone !== own)]
})

function openList(): void {
  void router.push('/automations')
}

function openAutomation(id: string): void {
  void router.push(`/automations/${encodeURIComponent(id)}`)
}

async function create(): Promise<void> {
  const id = await store.create()
  if (id) {
    openAutomation(id)
  }
}

async function remove(id: string): Promise<void> {
  await store.remove(id)
  openList()
}

function openRun(run: AutomationRun): void {
  if (run.conversationId) {
    navigation.open(run.conversationId)
  }
}
</script>

<template>
  <AutomationsPane
    :title="t('page.title')"
    :back="current ? t('page.title') : undefined"
    :add="t('page.new')"
    @back="openList"
    @add="create"
  >
    <AutomationEditor
      v-if="current"
      :key="current.id"
      :automation="current"
      :hosts="store.hosts"
      :conversations="store.conversations"
      :time-zones="timeZones"
      :providers="resources.providerInfos"
      :models="resources.models"
      :overlay-store="appOverlayStore"
      :secret="store.secrets[current.id] ?? null"
      :notifications-off="store.notificationsOff"
      @change="store.change(current.id, $event)"
      @back="openList"
      @run-now="store.runNow(current.id)"
      @delete="remove(current.id)"
      @show-secret="store.showSecret(current.id)"
      @hide-secret="store.hideSecret(current.id)"
      @regenerate-secret="store.showSecret(current.id, true)"
      @open-run="openRun"
      @turn-on-notifications="openSettings(router, 'notifications')"
    />
    <AutomationList
      v-else
      :automations="store.automations"
      :hosts="store.hosts"
      :conversations="store.conversations"
      :now="now"
      @open="openAutomation"
      @create="create"
    />
  </AutomationsPane>
</template>
