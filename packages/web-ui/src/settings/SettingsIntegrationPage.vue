<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import CopyCode from '../ui/CopyCode.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import InlineError from '../ui/InlineError.vue'
import Switch from '../ui/Switch.vue'
import TextArea from '../ui/TextArea.vue'
import type { SentenceText } from '../ui/ui-text'
import { shownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import type { Action } from '../ui/operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import {
  CAPABILITIES,
  CLOUD_DISKS,
  CLOUD_LIMITS,
  LIMIT_CHOICES,
  SIZE_UNITS,
  integrationSummary,
  limitAmount,
  limitChoice,
  parseSize,
  type CloudLimit,
  type CloudSizing,
  type LimitChoice,
  type ServerSizes,
  type IntegrationCapability,
  type IntegrationChange,
  type SettingsIntegration,
} from './integrations'

/**
 * An integration's page (`crates/backend-user-shard/src/integrations.rs`). It
 * shows:
 *
 * - its name, saved as it is committed;
 * - its token, whole with Copy only while the page holds the one its creation
 *   or Replace Token answered, hidden otherwise, since storage keeps only its
 *   hash; Replace Token asks first;
 * - a switch per capability, saved as it turns;
 * - its Clouds' init script, saved with Save, and the path of their shared
 *   folder, saved as it is committed;
 * - how long its Clouds stay awake without work, and their CPUs, memory and
 *   disks, which only an administrator changes and anyone else reads; an
 *   empty field is the server's, each saved as it is committed;
 * - Delete Integration, which asks first and names what goes.
 */
const props = defineProps<{
  integration: SettingsIntegration
  /** Its token as the request that made it answered; null once that answer is gone. */
  token: string | null
  /** What a Cloud gets for each size the integration leaves to the server; null until read. */
  serverSizes: ServerSizes | null
  overlayStore: OverlayStore
  /** Saves a change; a refusal the user can fix is thrown, and its field shows it. */
  onChange: (change: IntegrationChange) => Promise<void>
  onReplaceToken?: Action
  /** Delete Integration, once the user confirmed it. */
  onDelete?: Action
}>()

const emit = defineEmits<{
  back: []
}>()

/** Each field's refusal, under the field, until it is committed again. */
const problems = ref<Partial<Record<keyof IntegrationChange, SentenceText>>>({})

async function save(field: keyof IntegrationChange, change: IntegrationChange): Promise<void> {
  try {
    await props.onChange(change)
    problems.value = { ...problems.value, [field]: undefined }
  } catch (error) {
    problems.value = { ...problems.value, [field]: shownError(error).message }
  }
}

function nameProblem(value: string): SentenceText | null {
  return value.trim() ? null : t('integrations.page.nameProblem')
}

// The capabilities as the switches set them: a turn of one while another's
// save is under way keeps the other's, so no save carries an older set.
const wanted = ref({ ...props.integration.capabilities })
let saving = 0
watch(
  () => props.integration.capabilities,
  (saved) => {
    if (saving === 0)
      wanted.value = { ...saved }
  },
)
async function turn(capability: IntegrationCapability, on: boolean): Promise<void> {
  wanted.value = { ...wanted.value, [capability]: on }
  saving += 1
  try {
    await save('capabilities', { capabilities: { ...wanted.value } })
  } finally {
    saving -= 1
  }
}

function pathProblem(value: string): SentenceText | null {
  const path = value.trim()
  return path.startsWith('~/') || path.startsWith('/') ? null : t('integrations.page.sharedPathProblem')
}

// The script as typed, which follows the saved one until the user edits it.
const script = ref(props.integration.initScript ?? '')
let shownScript = script.value
watch(() => props.integration.initScript, (saved) => {
  if (script.value === shownScript)
    script.value = saved ?? ''
  shownScript = saved ?? ''
})
const scriptChanged = computed(() => script.value !== (props.integration.initScript ?? ''))
const savingScript = computed(() => ({ action: 'save', target: `integration-script:${props.integration.id}` }))
const scriptButton = ref<InstanceType<typeof Button> | null>(null)
async function saveScript(): Promise<void> {
  await save('initScript', { initScript: script.value.trim() ? script.value : null })
}

/** Each size's refusal, under its row, until it is committed again. */
const sizeProblems = ref<Partial<Record<keyof CloudSizing | 'idle', SentenceText>>>({})
/** A Cloud on the chosen isolation has no limits: the sizes stay, for another. */
const unenforced = computed(() => props.serverSizes?.limits === false)

function sizeProblem(value: string): SentenceText | null {
  return Number.isNaN(parseSize(value)) ? t('integrations.page.sizeProblem') : null
}

function sizeText(size: CloudLimit | null): string {
  return typeof size === 'number' ? String(size) : ''
}

async function saveIdle(value: string): Promise<void> {
  try {
    await props.onChange({ cloudIdleMinutes: parseSize(value) })
    sizeProblems.value = { ...sizeProblems.value, idle: undefined }
  } catch (error) {
    sizeProblems.value = { ...sizeProblems.value, idle: shownError(error).message }
  }
}

async function saveSize(size: keyof CloudSizing, value: CloudLimit | null): Promise<void> {
  try {
    await props.onChange({ cloudSizing: { ...props.integration.cloudSizing, [size]: value } })
    sizeProblems.value = { ...sizeProblems.value, [size]: undefined }
  } catch (error) {
    sizeProblems.value = { ...sizeProblems.value, [size]: shownError(error).message }
  }
}

type LimitSize = (typeof CLOUD_LIMITS)[number]

/** The limits set to Custom before the server said what they are, whose amount waits to be entered. */
const customLimits = ref<Partial<Record<LimitSize, boolean>>>({})

function shownChoice(size: LimitSize): LimitChoice {
  return customLimits.value[size] ? 'custom' : limitChoice(props.integration.cloudSizing[size])
}

/** What a limit's field shows: the amount a Cloud gets, which only Custom lets the user change. */
function limitText(size: LimitSize): string {
  const amount = customLimits.value[size] ? null : limitAmount(size, props.integration.cloudSizing[size], props.serverSizes)
  return amount === null ? '' : String(amount)
}

/** An empty size's placeholder: what the server gives. */
function serverText(size: 'systemGb' | 'homeGb' | 'sharedGb' | 'idleMinutes'): string {
  return props.serverSizes ? String(props.serverSizes[size]) : t('integrations.page.serverDefault')
}

function choiceLabel(choice: LimitChoice): string {
  return choice === 'default' ? t('integrations.page.serverDefault') : t(`integrations.page.limits.${choice}`)
}

async function chooseLimit(size: LimitSize, choice: LimitChoice): Promise<void> {
  // Custom starts at the amount the Cloud gets now, which the field then changes.
  const amount = choice === 'custom' ? limitAmount(size, props.integration.cloudSizing[size], props.serverSizes) : null
  customLimits.value = { ...customLimits.value, [size]: choice === 'custom' && amount === null }
  if (choice !== 'custom' || amount !== null)
    await saveSize(size, choice === 'unlimited' ? 'unlimited' : amount)
}

/** A custom amount; an empty field goes back to the server's. */
async function saveLimit(size: LimitSize, value: string): Promise<void> {
  customLimits.value = { ...customLimits.value, [size]: false }
  await saveSize(size, parseSize(value))
}

const deleteOpen = ref(false)
const deleting = computed(() => ({ action: 'delete', target: `integration:${props.integration.id}` }))
async function remove(): Promise<void> {
  try {
    await props.onDelete?.()
  } finally {
    deleteOpen.value = false
  }
}

const replaceOpen = ref(false)
const replacing = computed(() => ({ action: 'replace', target: `integration-token:${props.integration.id}` }))
async function replace(): Promise<void> {
  try {
    await props.onReplaceToken?.()
  } finally {
    replaceOpen.value = false
  }
}
</script>

<template>
  <SettingsPage
    :title="integration.name"
    :back="t('integrations.title')"
    @back="emit('back')"
  >
    <template #description>
      <p>{{ integrationSummary(integration) }}</p>
    </template>
    <SettingsGroup>
      <SettingsRow :label="t('integrations.page.name')">
        <template v-if="problems.name" #detail>
          <InlineError :message="problems.name" />
        </template>
        <CommitTextInput
          :model-value="integration.name"
          :aria-label="t('integrations.page.name')"
          :validate="nameProblem"
          trim
          class="w-56 max-w-full"
          @problem="problems.name = $event ?? undefined"
          @commit="save('name', { name: $event.trim() })"
        />
      </SettingsRow>
      <SettingsRow :label="t('integrations.page.token')" :description="t('integrations.page.tokenDescription')">
        <template v-if="token" #detail>
          <div class="flex min-w-0 flex-col gap-2">
            <p class="text-[12px] leading-4 text-fg-muted">{{ t('integrations.page.tokenNew') }}</p>
            <CopyCode :code="token" />
          </div>
        </template>
        <div class="flex items-center gap-3">
          <span v-if="!token" class="text-[13px] text-fg-subtle">{{ t('integrations.page.tokenHidden') }}</span>
          <Button size="sm" :operation="replacing" @click="replaceOpen = true">{{ t('integrations.page.replace') }}</Button>
        </div>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      :title="t('integrations.page.capabilities')"
      :description="t('integrations.page.capabilitiesDescription')"
    >
      <template v-if="problems.capabilities" #actions>
        <InlineError :message="problems.capabilities" />
      </template>
      <SettingsRow
        v-for="capability in CAPABILITIES"
        :key="capability"
        :label="t(`integrations.page.${capability}`)"
        :description="t(`integrations.page.${capability}Description`)"
      >
        <Switch
          :model-value="wanted[capability]"
          :aria-label="t(`integrations.page.${capability}`)"
          size="sm"
          @update:model-value="turn(capability, $event)"
        />
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      :title="t('integrations.page.initScript')"
      :description="t('integrations.page.initScriptDescription')"
    >
      <template #actions>
        <InlineError v-if="problems.initScript" :message="problems.initScript" />
        <Button
          ref="scriptButton"
          size="sm"
          variant="primary"
          :operation="savingScript"
          :disabled="!scriptChanged"
          @click="saveScript"
        >
          {{ t('integrations.page.save') }}
        </Button>
      </template>
      <div class="p-3">
        <TextArea
          v-model="script"
          :rows="8"
          :aria-label="t('integrations.page.initScript')"
          :placeholder="t('integrations.page.initScriptPlaceholder')"
          mono
          spellcheck="false"
          @keydown.meta.enter.prevent="scriptButton?.press()"
          @keydown.ctrl.enter.prevent="scriptButton?.press()"
        />
      </div>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('integrations.page.sharedPath')" :description="t('integrations.page.sharedPathDescription')">
        <template v-if="problems.sharedPath" #detail>
          <InlineError :message="problems.sharedPath" />
        </template>
        <CommitTextInput
          :model-value="integration.sharedPath"
          :aria-label="t('integrations.page.sharedPath')"
          :validate="pathProblem"
          trim
          class="w-56 max-w-full"
          @problem="problems.sharedPath = $event ?? undefined"
          @commit="save('sharedPath', { sharedPath: $event.trim() })"
        />
      </SettingsRow>
      <SettingsRow :label="t('integrations.page.sharedGb')" :description="unenforced ? t('integrations.page.sharedGbUnenforced') : t('integrations.page.sharedGbDescription')">
        <template v-if="sizeProblems.sharedGb" #detail>
          <InlineError :message="sizeProblems.sharedGb" />
        </template>
        <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
          <CommitTextInput
            :model-value="sizeText(integration.cloudSizing.sharedGb)"
            :aria-label="t('integrations.page.sharedGb')"
            :placeholder="serverText('sharedGb')"
            :disabled="unenforced"
            :validate="sizeProblem"
            inputmode="numeric"
            literal
            trim
            class="w-32"
            @problem="sizeProblems.sharedGb = $event ?? undefined"
            @commit="saveSize('sharedGb', parseSize($event))"
          />
          <span class="w-14">{{ t('integrations.page.gbUnit') }}</span>
        </span>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      :title="t('integrations.page.clouds')"
      :description="unenforced ? t('integrations.page.cloudsUnenforced') : t('integrations.page.cloudsDescription')"
    >
      <SettingsRow :label="t('integrations.page.idle')" :description="t('integrations.page.idleDescription')">
        <template v-if="sizeProblems.idle" #detail>
          <InlineError :message="sizeProblems.idle" />
        </template>
        <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
          <CommitTextInput
            :model-value="sizeText(integration.cloudIdleMinutes)"
            :aria-label="t('integrations.page.idle')"
            :placeholder="serverText('idleMinutes')"
            :validate="sizeProblem"
            inputmode="numeric"
            literal
            trim
            class="w-32"
            @problem="sizeProblems.idle = $event ?? undefined"
            @commit="saveIdle"
          />
          <span class="w-14">{{ t('integrations.page.minutes') }}</span>
        </span>
      </SettingsRow>
      <SettingsRow
        v-for="size in CLOUD_LIMITS"
        :key="size"
        :label="t(`integrations.page.${size}`)"
      >
        <template v-if="sizeProblems[size]" #detail>
          <InlineError :message="sizeProblems[size]" />
        </template>
        <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
          <!-- One width for every choice, so the field doesn't move. -->
          <span class="inline-flex w-28">
            <Dropdown
              size="sm"
              :overlay-store="overlayStore"
              variant="default"
              width="fill"
              :disabled="unenforced"
              :trigger-label="t(`integrations.page.${size}`)"
            >
              <template #trigger>{{ choiceLabel(shownChoice(size)) }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="choice in LIMIT_CHOICES"
                    :key="choice"
                    :label="choiceLabel(choice)"
                    choice
                    :is-selected="shownChoice(size) === choice"
                    @select="close(); choice === shownChoice(size) || chooseLimit(size, choice)"
                  />
                </Menu>
              </template>
            </Dropdown>
          </span>
          <CommitTextInput
            :model-value="limitText(size)"
            :aria-label="t(`integrations.page.${size}`)"
            :disabled="unenforced || shownChoice(size) !== 'custom'"
            :validate="sizeProblem"
            inputmode="numeric"
            literal
            trim
            class="w-24"
            @problem="sizeProblems[size] = $event ?? undefined"
            @commit="saveLimit(size, $event)"
          />
          <span class="w-14">{{ SIZE_UNITS[size] ? t(`integrations.page.${SIZE_UNITS[size]}`) : '' }}</span>
        </span>
      </SettingsRow>
      <SettingsRow
        v-for="size in CLOUD_DISKS"
        :key="size"
        :label="t(`integrations.page.${size}`)"
      >
        <template v-if="sizeProblems[size]" #detail>
          <InlineError :message="sizeProblems[size]" />
        </template>
        <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
          <CommitTextInput
            :model-value="sizeText(integration.cloudSizing[size])"
            :aria-label="t(`integrations.page.${size}`)"
            :placeholder="serverText(size)"
            :disabled="unenforced"
            :validate="sizeProblem"
            inputmode="numeric"
            literal
            trim
            class="w-32"
            @problem="sizeProblems[size] = $event ?? undefined"
            @commit="saveSize(size, parseSize($event))"
          />
          <span class="w-14">{{ SIZE_UNITS[size] ? t(`integrations.page.${SIZE_UNITS[size]}`) : '' }}</span>
        </span>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('integrations.page.delete')" :description="t('integrations.page.deleteDescription')">
        <Button size="sm" variant="danger" :operation="deleting" @click="deleteOpen = true">{{ t('integrations.page.deleteButton') }}</Button>
      </SettingsRow>
    </SettingsGroup>

    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="t('integrations.page.deleteTitle', { name: integration.name })"
      :action="t('integrations.page.deleteConfirm')"
      :operation="deleting"
      :on-confirm="remove"
      @close="deleteOpen = false"
    >
      <p>{{ t('integrations.page.deleteBody', { clouds: integrationSummary(integration) }) }}</p>
    </ConfirmDialog>

    <ConfirmDialog
      :is-open="replaceOpen"
      :overlay-store="overlayStore"
      :title="t('integrations.page.replaceTitle')"
      :action="t('integrations.page.replaceConfirm')"
      :operation="replacing"
      :on-confirm="replace"
      @close="replaceOpen = false"
    >
      <p>{{ t('integrations.page.replaceBody') }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
