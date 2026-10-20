<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { t as shared } from '../locales/text'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import TextInput from '../ui/TextInput.vue'
import { t } from './locales/text'
import type { Repeat, RepeatUnit } from './types'

/**
 * Custom…, as Calendar's custom repeat: a sheet that sets any cycle, its
 * count, its unit and, for a cycle of several units, the day its first cycle
 * starts on. Done hands the cycle on; Cancel drops it.
 */
const props = defineProps<{
  isOpen: boolean
  /** The cycle the sheet starts from. */
  repeat: Repeat
  /** Today by the automation's clocks, the first cycle's day unless the cycle has one. */
  today: string
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  done: [repeat: Repeat]
  close: []
}>()

const count = ref('')
const unit = ref<RepeatUnit>('weeks')
const start = ref('')
watch(() => props.isOpen, (open) => {
  if (open) {
    count.value = String(props.repeat.every)
    unit.value = props.repeat.unit
    start.value = props.repeat.start ?? props.today
  }
}, { immediate: true })

const DAY = /^(\d{4})-(\d{2})-(\d{2})$/
const every = computed(() => (/^[1-9]\d{0,2}$/.test(count.value.trim()) ? Number(count.value.trim()) : null))
const startValid = computed(() => {
  const match = DAY.exec(start.value.trim())
  if (!match) {
    return false
  }
  const [year, month, day] = match.slice(1).map(Number) as [number, number, number]
  const date = new Date(Date.UTC(year, month - 1, day))
  return date.getUTCMonth() === month - 1 && date.getUTCDate() === day
})
const problem = computed(() => {
  if (every.value === null) {
    return t('trigger.intervalInvalid')
  }
  return every.value > 1 && !startValid.value ? t('custom.startInvalid') : null
})
const units = computed(() => (['days', 'weeks', 'months', 'years'] as const)
  .map((value) => ({ value, label: t(`custom.units.${value}`, {}, every.value ?? 2) })))

function done(): void {
  if (every.value !== null && problem.value === null) {
    emit('done', { every: every.value, unit: unit.value, start: every.value > 1 ? start.value.trim() : null })
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="t('custom.title')" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <h3 class="select-none pr-8 text-[15px] font-medium text-fg-emphasis">{{ t('custom.title') }}</h3>
      <SettingsGroup>
        <SettingsRow :label="t('custom.countLead')" :description="problem ?? undefined">
          <div class="flex items-center gap-2">
            <TextInput v-model="count" class="w-16" literal :aria-label="t('custom.count')" />
            <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('custom.unit')">
              <template #trigger>{{ units.find((each) => each.value === unit)?.label }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="each in units"
                    :key="each.value"
                    :label="each.label"
                    choice
                    :is-selected="each.value === unit"
                    @select="unit = each.value; close()"
                  />
                </Menu>
              </template>
            </Dropdown>
          </div>
        </SettingsRow>
        <!-- One unit a cycle has no first cycle to name. -->
        <SettingsRow v-if="every !== null && every > 1" :label="t('custom.starts')">
          <TextInput v-model="start" class="w-28" literal :aria-label="t('custom.starts')" />
        </SettingsRow>
      </SettingsGroup>
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ shared('common.cancel') }}</Button>
      <Button variant="primary" :disabled="problem !== null" @click="done">{{ t('tasks.done') }}</Button>
    </template>
  </Dialog>
</template>
