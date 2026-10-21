<script setup lang="ts">
import { requestFailure } from '../fixtures/request-failures'
import type { ShownError } from '@demicodes/web-ui/infra/errors'
import { computed, ref } from 'vue'
import { Monitor, Moon, RefreshCw, Sun } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import ProviderQuota from '@demicodes/web-ui/settings/ProviderQuota.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import type { SettingsQuotaWindow } from '@demicodes/web-ui/settings/types'
import Button from '@demicodes/web-ui/ui/Button.vue'
import CommitTextInput from '@demicodes/web-ui/ui/CommitTextInput.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import LimitField from '@demicodes/web-ui/ui/LimitField.vue'
import type { LimitValue } from '@demicodes/web-ui/ui/limit-field'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Segmented, { type SegmentedOption } from '@demicodes/web-ui/ui/Segmented.vue'
import Slider from '@demicodes/web-ui/ui/Slider.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { DEFAULT_TRANSCRIPT_TEXT_SIZE } from '@demicodes/web-ui/theme/productAppearance'
import { productWould } from '../product-would'
import { DAY, HOUR, MINUTE, ago, ahead } from '../fixtures/time'
import { HOUR_SECONDS, WEEK_SECONDS, windowNamed } from '../fixtures/usage-limits'

/**
 * One row of each kind of control a settings card holds, as the product's
 * pages place them. Each control acts on the specimen's own state.
 */
type Theme = 'light' | 'dark' | 'system'

/**
 * The rows in English, or in Japanese, whose longer labels show where a row
 * puts its control under the text instead of beside it (SettingsRow).
 */
const props = defineProps<{ ja?: boolean }>()
const EN = {
  group: 'Every Control', name: 'Name', nameHint: 'Lowercase letters, digits and hyphens.',
  baseUrl: 'Base URL', retries: 'Retries', retriesHint: 'A number, or Unlimited after a warning.',
  confirmTitle: 'Allow unlimited retries?', confirmAction: 'Allow Unlimited',
  confirmBody: 'A request that keeps failing is then tried again without end.',
  textSize: 'Message text size', protocol: 'Protocol', theme: 'Theme',
  light: 'Light', dark: 'Dark', system: 'System',
  sound: 'Sound', soundHint: 'When a turn finishes in the background.',
  export: 'Export', exportHint: 'A copy of every conversation, sent by email.', requestExport: 'Request Export',
  requestExportWould: 'Request an Export', refresh: 'Refresh usage',
}
const JA: typeof EN = {
  group: 'すべてのコントロール', name: '名前', nameHint: '小文字の英字、数字、ハイフン。',
  baseUrl: 'ベース URL', retries: '再試行', retriesHint: '数値、または警告のあとに無制限。',
  confirmTitle: '無制限の再試行を許可しますか?', confirmAction: '無制限を許可',
  confirmBody: '失敗し続けるリクエストは、終わりなく再試行されます。',
  textSize: 'メッセージのフォントサイズ', protocol: 'プロトコル', theme: 'テーマ',
  light: 'ライト', dark: 'ダーク', system: 'システム',
  sound: 'サウンド', soundHint: 'バックグラウンドでターンが完了したとき。',
  export: '書き出し', exportHint: 'すべての会話のコピーをメールで送信します。', requestExport: '書き出しを依頼',
  requestExportWould: '書き出しを依頼', refresh: '使用量を更新',
}
const c = computed(() => (props.ja ? JA : EN))
const themes = computed<readonly SegmentedOption<Theme>[]>(() => [
  { value: 'light', label: c.value.light, icon: Sun },
  { value: 'dark', label: c.value.dark, icon: Moon },
  { value: 'system', label: c.value.system, icon: Monitor },
])
const protocols = ['Chat Completions', 'Responses', 'Messages'] as const

const name = ref('explore')
const baseUrl = ref('https://api.example.com/v1')
const retries = ref<LimitValue>({ limit: 3, unlimited: false })
const retriesProblem = ref<string | null>(null)
const protocol = ref<(typeof protocols)[number]>('Responses')
const fontSize = ref(DEFAULT_TRANSCRIPT_TEXT_SIZE)
const theme = ref<Theme>('system')
const sound = ref(true)
/** The account row's usage, which Refresh reads again: each window a little further used. */
const usage = ref<SettingsQuotaWindow[]>([
  { id: 'five_hour', name: windowNamed('limit', 5 * HOUR_SECONDS), used: 62, max: 100, resetsAt: ahead(130 * MINUTE) },
  { id: 'seven_day', name: windowNamed('limit', WEEK_SECONDS), used: 31, max: 100, resetsAt: ahead(3 * DAY + 4 * HOUR) },
])
/** When the vendor reported the usage. */
const usageAt = ref(ago(3 * MINUTE))
/** The vendor's words for a refresh that failed, until the next answer. */
const usageFailure = ref<ShownError | null>(null)
/** Every other press fails, so the row shows both outcomes. */
let presses = 0
function refreshUsage(): void {
  if (++presses % 2 === 1) {
    usageFailure.value = requestFailure('Anthropic’s usage service is unavailable.')
    return
  }
  usageFailure.value = null
  usage.value = usage.value.map((window) => ({ ...window, used: Math.min(window.max, window.used + 3) }))
  usageAt.value = new Date().toISOString()
}
</script>

<template>
  <SettingsGroup :title="c.group">
    <SettingsRow :label="c.name" :description="c.nameHint">
      <TextInput v-model="name" :aria-label="c.name" class="w-48 max-w-full" />
    </SettingsRow>
    <SettingsRow :label="c.baseUrl">
      <CommitTextInput
        :model-value="baseUrl"
        :aria-label="c.baseUrl"
        class="w-72 max-w-full"
        @commit="baseUrl = $event"
      />
    </SettingsRow>
    <SettingsRow :label="c.retries" :description="c.retriesHint">
      <template v-if="retriesProblem" #detail>
        <InlineError :message="retriesProblem" />
      </template>
      <LimitField
        :model-value="retries"
        :label="c.retries"
        :overlay-store="appOverlayStore"
        :confirm-title="c.confirmTitle"
        :confirm-action="c.confirmAction"
        @commit="retries = $event"
        @problem="retriesProblem = $event"
      >
        <p>{{ c.confirmBody }}</p>
      </LimitField>
    </SettingsRow>
    <SettingsRow :label="c.textSize">
      <Slider
        v-model="fontSize"
        :min="12"
        :max="18"
        :value-label="`${fontSize}px`"
        class="w-48"
      />
    </SettingsRow>
    <SettingsRow :label="c.protocol">
      <Dropdown
        size="sm"
        :overlay-store="appOverlayStore"
        variant="default"
        :trigger-label="c.protocol"
      >
        <template #trigger>{{ protocol }}</template>
        <template #content="{ close }">
          <Menu>
            <MenuItem
              v-for="entry in protocols"
              :key="entry"
              :label="entry"
              choice
              :is-selected="protocol === entry"
              @select="protocol = entry; close()"
            />
          </Menu>
        </template>
      </Dropdown>
    </SettingsRow>
    <SettingsRow :label="c.theme">
      <Segmented v-model="theme" size="sm" :options="themes" />
    </SettingsRow>
    <SettingsRow :label="c.sound" :description="c.soundHint">
      <Switch v-model="sound" :aria-label="c.sound" />
    </SettingsRow>
    <SettingsRow :label="c.export" :description="c.exportHint">
      <Button size="sm" @click="productWould(c.requestExportWould)">{{ c.requestExport }}</Button>
    </SettingsRow>
    <SettingsRow label="alex@example.com">
      <template #tags><Tag>Max 5×</Tag></template>
      <Tooltip :content="c.refresh">
        <IconButton size="sm" :icon="RefreshCw" :aria-label="c.refresh" spin-on-click @click="refreshUsage" />
      </Tooltip>
      <template #below>
        <ProviderQuota :windows="usage" :observed-at="usageAt" :error="usageFailure" />
      </template>
    </SettingsRow>
  </SettingsGroup>
</template>
