<script setup lang="ts">
import { computed } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, CircleX, Copy } from '@lucide/vue'
import type { ProviderErrorDiagnostics } from '@demicodes/core'
import { t } from '@demicodes/web-ui/infra/i18n'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { errorFacts, errorReportText, errorSummary } from '../error-detail'
import FunctionalBlock from './FunctionalBlock.vue'

const props = defineProps<{
  /** The upstream error as the provider reported it. */
  message: string
  code?: string | null
  diagnostics?: ProviderErrorDiagnostics
}>()

const summary = computed(() => errorSummary(props.code))
const facts = computed(() => errorFacts(props.code, props.diagnostics))
const reportText = computed(() => errorReportText(props.message, props.code, props.diagnostics))
const { copy, copied } = useClipboard({ copiedDuring: 1500 })
</script>

<template>
  <FunctionalBlock expandable tone="danger">
    <template #icon>
      <CircleX :size="ICON_PX.in28" />
    </template>
    <span class="min-w-0 truncate">{{ summary }}</span>
    <template #body>
      <div class="flex items-start gap-2 px-3 py-1.5">
        <div class="min-w-0 flex-1 select-text text-xs leading-5">
          <p class="whitespace-pre-wrap break-words text-on-danger">{{ message }}</p>
          <p v-if="facts.length > 0" class="mt-1 truncate font-mono text-fg-subtle">{{ facts.join(' · ') }}</p>
        </div>
        <Tooltip :content="copied ? t('common.copied') : t('common.copy')">
          <IconButton
            :icon="copied ? Check : Copy"
            size="sm"
            variant="ghost"
            @click.stop="copy(reportText)"
          />
        </Tooltip>
      </div>
    </template>
  </FunctionalBlock>
</template>
