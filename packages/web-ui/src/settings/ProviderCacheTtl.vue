<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import CommitTextInput from '../ui/CommitTextInput.vue'
import Dropdown from '../ui/Dropdown.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'
import SettingsRow from './SettingsRow.vue'
import { CACHE_TTL_PATTERN, cacheTtlLabel, type SettingsCacheTtl } from './types'

/**
 * An API key's Prompt cache lifetime: Unspecified, which sends none, one of
 * the lifetimes its protocol knows, or Custom…, which takes any lifetime in a
 * field beside the menu, such as `2h` or `90m`.
 *
 * Choosing Custom… opens the field without changing anything; the value
 * changes when the field commits one. A value the field cannot commit stays
 * in it, said under the row, until it is fixed or Escape gives it back.
 *
 * The page keys it by its entry, so another entry's starts from its own value.
 */
const props = defineProps<{
  cacheTtl: SettingsCacheTtl
  overlayStore: OverlayStore
  disabled?: boolean
}>()

const emit = defineEmits<{
  /** The setting with the lifetime to ask for, null for none. */
  change: [cacheTtl: SettingsCacheTtl]
}>()

/** Whether the stored lifetime is none of the presets. */
const storedCustom = computed(() =>
  props.cacheTtl.value !== null && !props.cacheTtl.presets.includes(props.cacheTtl.value),
)
/** Whether the field shows: for a custom lifetime, or after Custom… until another choice. */
const customOpen = ref(storedCustom.value)
watch(storedCustom, (custom) => {
  customOpen.value = custom
})

/** Whether the user just chose Custom…, so the field that opens takes the focus. */
const chosen = ref(false)
/** What is wrong with the lifetime kept in the field. */
const problem = ref<SentenceText | null>(null)

const triggerLabel = computed(() => {
  if (customOpen.value) return t('providers.cacheTtl.custom')
  const value = props.cacheTtl.value
  return value === null ? t('providers.cacheTtl.unspecified') : cacheTtlLabel(value)
})

function choose(value: string | null): void {
  customOpen.value = false
  chosen.value = false
  problem.value = null
  if (value !== props.cacheTtl.value) emit('change', { ...props.cacheTtl, value })
}

function chooseCustom(): void {
  chosen.value = !customOpen.value
  customOpen.value = true
}

function check(value: string) {
  return CACHE_TTL_PATTERN.test(value.trim()) ? null : t('providers.cacheTtl.invalid')
}

function commit(value: string): void {
  const typed = value.trim()
  if (typed && typed !== props.cacheTtl.value) emit('change', { ...props.cacheTtl, value: typed })
}
</script>

<template>
  <SettingsRow :label="t('providers.cacheTtl.title')" :description="t('providers.cacheTtl.description')">
    <template v-if="customOpen && problem" #detail>
      <InlineError :message="problem" />
    </template>
    <div class="flex min-w-0 items-center gap-2">
      <Dropdown
        size="sm"
        :overlay-store="overlayStore"
        variant="default"
        :trigger-label="t('providers.cacheTtl.title')"
        :disabled="disabled"
      >
        <template #trigger>{{ triggerLabel }}</template>
        <template #content="{ close }">
          <Menu>
            <MenuItem
              :label="t('providers.cacheTtl.unspecified')"
              choice
              :is-selected="!customOpen && cacheTtl.value === null"
              @select="close(); choose(null)"
            />
            <MenuDivider />
            <MenuItem
              v-for="preset in cacheTtl.presets"
              :key="preset"
              :label="cacheTtlLabel(preset)"
              choice
              :is-selected="!customOpen && cacheTtl.value === preset"
              @select="close(); choose(preset)"
            />
            <MenuDivider />
            <MenuItem
              :label="t('providers.cacheTtl.customItem')"
              choice
              :is-selected="customOpen"
              @select="close(); chooseCustom()"
            />
          </Menu>
        </template>
      </Dropdown>
      <CommitTextInput
        v-if="customOpen"
        :model-value="storedCustom ? cacheTtl.value ?? '' : ''"
        :aria-label="t('providers.cacheTtl.customField')"
        :placeholder="t('providers.cacheTtl.customPlaceholder')"
        :disabled="disabled"
        :validate="check"
        :focused="chosen"
        asked
        trim
        class="w-[5.5rem]"
        @problem="problem = $event"
        @commit="commit"
      />
    </div>
  </SettingsRow>
</template>
