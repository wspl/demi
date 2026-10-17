import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { conversationTitle } from '@demicodes/web-ui/agent/conversation-title'
import { t } from '@demicodes/web-ui/automations/locales/text'
import type {
  Automation,
  AutomationChange,
  AutomationConversation,
  AutomationHost,
  AutomationLink,
  AutomationTarget,
} from '@demicodes/web-ui/automations/types'
import { triggerWords } from '@demicodes/web-ui/automations/words'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  automationAnswerSchema,
  automationSecretSchema,
  type AutomationDto,
  type AutomationPatch,
  type AutomationTarget as TargetDto,
  type ConversationTarget,
} from '../api/generated/web-api'
import { cloudDeviceIn, deviceMarkOf, hostKindOf, hostLabel } from '../state/hosts'
import { useProduct } from '../state/product'

/** The Host a target names before the first use made the Cloud a device. */
const CLOUD = 'cloud'

/** The automations in their order, `automation` in its place by id or, new, at the front. */
function withAutomation(automations: AutomationDto[], automation: AutomationDto): AutomationDto[] {
  const index = automations.findIndex((candidate) => candidate.id === automation.id)
  return index >= 0 ? automations.toSpliced(index, 1, automation) : [automation, ...automations]
}

/**
 * The user's automations as the Automations page shows and edits them
 * (`automations.md` § Where the user sees them): the product state's
 * `automations` part, which the synchronization channel brings to every
 * page, read in the page's shapes; each edit shows at once and is written
 * after the earlier ones of the same automation.
 */
export const useAutomations = defineStore('automations', () => {
  const product = useProduct()
  const writes = new SerialQueue()
  /** The secret Show shows, by automation; the page keeps none it does not show. */
  const secrets = ref<Record<string, string>>({})

  const devices = computed(() => product.snapshot?.devices ?? [])
  const workspaces = computed(() => product.snapshot?.workspaces ?? [])
  const cloudId = computed(() => cloudDeviceIn(devices.value)?.id ?? CLOUD)

  function hostIdOf(target: ConversationTarget): string {
    switch (target.kind) {
      case 'cloud':
        return cloudId.value
      case 'device':
        return target.deviceId
      case 'workspace':
        return workspaces.value.find((workspace) => workspace.id === target.workspaceId)?.deviceId ?? cloudId.value
    }
  }

  function targetOf(target: TargetDto): AutomationTarget {
    if (target.kind === 'existing') {
      return { kind: 'existing', conversationId: target.conversationId }
    }
    const place = target.target
    return { kind: 'new', hostId: hostIdOf(place), projectId: place.kind === 'workspace' ? place.workspaceId : null }
  }

  /** A target as the web API names it: a project's workspace, the Cloud, or a device's home directory. */
  function targetDto(target: AutomationTarget): TargetDto {
    if (target.kind === 'existing') {
      return target
    }
    if (target.projectId !== null) {
      return { kind: 'new', target: { kind: 'workspace', workspaceId: target.projectId } }
    }
    const device = devices.value.find((candidate) => candidate.id === target.hostId)
    if (!device || hostKindOf(device) === 'cloud' || device.home === null) {
      return { kind: 'new', target: { kind: 'cloud' } }
    }
    return { kind: 'new', target: { kind: 'device', deviceId: device.id, path: device.home } }
  }

  function automationOf(dto: AutomationDto): Automation {
    return {
      id: dto.id,
      name: dto.name,
      trigger: dto.trigger,
      timeZone: dto.timeZone,
      maxRunsPerDay: dto.maxRunsPerDay,
      target: targetOf(dto.target),
      prompt: dto.prompt,
      model: dto.model,
      paused: dto.paused,
      pausedBecause: dto.pausedBecause,
      address: dto.trigger.kind === 'url' ? `${window.location.origin}/hooks/${dto.id}` : null,
      stripeSigning: dto.stripeSigning,
      nextAt: dto.nextAt,
      runs: dto.runs,
    }
  }

  const automations = computed<Automation[]>(() => (product.snapshot?.automations ?? []).map(automationOf))

  /** The Hosts a target can name: the Cloud first, then the paired devices, each with its projects. */
  const hosts = computed<AutomationHost[]>(() => {
    const listed = devices.value.map((device): AutomationHost => ({
      id: device.id,
      name: hostLabel(hostKindOf(device), device),
      kind: hostKindOf(device),
      state: device.state,
      mark: deviceMarkOf(device, devices.value),
      projects: workspaces.value
        .filter((workspace) => workspace.deviceId === device.id)
        .map((workspace) => ({ id: workspace.id, name: workspace.name })),
    }))
    const cloud = listed.filter((host) => host.kind === 'cloud')
    const others = listed.filter((host) => host.kind !== 'cloud')
    return cloud.length > 0
      ? [...cloud, ...others]
      : [{ id: CLOUD, name: hostLabel('cloud', null), kind: 'cloud', state: 'online', mark: null, projects: [] }, ...others]
  })

  /** The conversations an automation can wake: the sidebar's, with their Hosts. */
  const conversations = computed<AutomationConversation[]>(() =>
    (product.snapshot?.conversations ?? [])
      .filter((conversation) => !conversation.archived && conversation.automation === null)
      .map((conversation) => ({
        id: conversation.id,
        title: conversationTitle(conversation.title),
        hostId: hostIdOf(conversation.target),
      })),
  )

  /**
   * No browser of the user has notifications on, which each session says
   * and the sessions part announces as it changes: what runs notify reaches
   * nobody.
   */
  const notificationsOff = computed(() =>
    product.snapshot !== null && !product.snapshot.sessions.some((session) => session.notifications))

  /** The sidebar's count: runs that notified and whose conversation was not opened since. */
  const unread = computed(() => product.snapshot?.automationsUnread ?? 0)

  /** The automations that wake the conversation `id`, as its chip and its row's mark list them. */
  function wakers(id: string): AutomationLink[] {
    return automations.value
      .filter((automation) => automation.target.kind === 'existing' && automation.target.conversationId === id)
      .map((automation) => ({ id: automation.id, name: automation.name, trigger: triggerWords(automation), paused: automation.paused }))
  }

  /** The automation and run a filed conversation belongs to. */
  function runOf(filed: { automationId: string; runId: string; at: string }) {
    // The conversation's record names its run, of any age; the automation
    // is always listed, its runs only the latest.
    const automation = automations.value.find((candidate) => candidate.id === filed.automationId)
    return automation ? { automation, run: { id: filed.runId, at: filed.at } } : null
  }


  /** New Automation: one at once, with the backend's defaults; answers its id. */
  async function create(): Promise<string | null> {
    const at = product.sent()
    try {
      const response = await apiRequest('/automations', { method: 'POST', ...jsonBody({} satisfies AutomationPatch) })
      const { automation } = await readResponse(response, automationAnswerSchema)
      product.answered(at, (state) => ({ type: 'automations', automations: withAutomation(state.automations, automation), automationsUnread: state.automationsUnread }))
      return automation.id
    } catch (error) {
      reportError(t('failed.create'), error, { userVisible: true })
      return null
    }
  }

  /** The part of the web API's patch `change` makes. */
  function patchOf(change: AutomationChange): AutomationPatch {
    const { target, ...rest } = change
    return target ? { ...rest, target: targetDto(target) } : rest
  }

  /** Shows `change` at once and writes it after the automation's earlier changes. */
  function change(id: string, change: AutomationChange): Promise<void> {
    const patch = patchOf(change)
    const { stripeSecret, ...shown } = patch
    const pending = product.change('automations', (state) => ({
      ...state,
      automations: state.automations.map((automation) => automation.id === id
        ? {
            ...automation,
            ...shown,
            ...(stripeSecret === undefined ? {} : { stripeSigning: stripeSecret !== null }),
            ...(shown.paused === false ? { pausedBecause: null } : {}),
          }
        : automation),
    }))
    return writes
      .run(id, async () => {
        pending.send()
        const response = await apiRequest(`/automations/${encodeURIComponent(id)}`, { method: 'PATCH', ...jsonBody(patch) })
        const { automation } = await readResponse(response, automationAnswerSchema)
        pending.land((state) => ({ type: 'automations', automations: withAutomation(state.automations, automation), automationsUnread: state.automationsUnread }))
      })
      .catch((error) => {
        pending.drop()
        reportError(t('failed.save'), error, { userVisible: true })
      })
  }

  /** Run Now: a run starts, whatever the limit, paused or not. */
  async function runNow(id: string): Promise<void> {
    try {
      await apiRequest(`/automations/${encodeURIComponent(id)}/run`, { method: 'POST' })
    } catch (error) {
      reportError(t('failed.run'), error, { userVisible: true })
    }
  }

  async function remove(id: string): Promise<void> {
    const at = product.sent()
    try {
      await apiRequest(`/automations/${encodeURIComponent(id)}`, { method: 'DELETE' })
      product.answered(at, (state) => ({ type: 'automations', automations: state.automations.filter((automation) => automation.id !== id), automationsUnread: state.automationsUnread }))
    } catch (error) {
      reportError(t('failed.delete'), error, { userVisible: true })
    }
  }

  /** Show: reads the URL's secret; Regenerate makes a new one, which then shows. */
  async function showSecret(id: string, regenerate = false): Promise<void> {
    try {
      const response = await apiRequest(`/automations/${encodeURIComponent(id)}/secret`, regenerate ? { method: 'POST' } : {})
      const { secret } = await readResponse(response, automationSecretSchema)
      secrets.value = { ...secrets.value, [id]: secret }
    } catch (error) {
      reportError(regenerate ? t('failed.regenerate') : t('failed.secret'), error, { userVisible: true })
    }
  }

  function hideSecret(id: string): void {
    const { [id]: _hidden, ...others } = secrets.value
    secrets.value = others
  }

  /** Move to Conversations: the run's conversation joins the sidebar. */
  async function moveToConversations(conversationId: string): Promise<void> {
    try {
      await apiRequest(`/conversations/${encodeURIComponent(conversationId)}/move-to-conversations`, { method: 'POST' })
    } catch (error) {
      reportError(t('failed.move'), error, { userVisible: true })
    }
  }

  return {
    automations,
    hosts,
    conversations,
    unread,
    notificationsOff,
    secrets,
    wakers,
    runOf,
    create,
    change,
    runNow,
    remove,
    showSecret,
    hideSecret,
    moveToConversations,
  }
})
