<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { Brain, Cpu, Layers, Zap } from '@lucide/vue'
import type { Component } from 'vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import type { TitleText } from '../ui/ui-text'
import type { InlineDetailText } from '../ui/inline-detail'
import type { ContextLimitChange } from './context-limit'
import { modelSwitch, type ModelSettings, type ModelSettingsChange } from './model-selection'
import { modelMenuRows } from './model-menu'
import ContextMenu from './ContextMenu.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'

const props = defineProps<{
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  /**
   * The conversation's model settings; none while nothing is chosen, or
   * while Parent's model is, when the menu offers it.
   */
  settings?: ModelSettings | null
  /**
   * Offers a choice of no model of its own at the top, chosen while
   * `settings` is none: Parent's Model for a subagent profile, whose
   * children may infer with their parent's model, or Automatic for the
   * small model, with what it resolves to as its detail when that is known.
   */
  standIn?: { label: TitleText; icon: Component; detail?: InlineDetailText }
  /**
   * Leaves out the Context row, the user's limit, as a subagent profile's
   * editor does, whose Context is the profile's own.
   */
  withoutContext?: boolean
  /** Leaves out Fast Mode, as the small model's menu does, whose requests use the default tier. */
  withoutFast?: boolean
}>()

const emit = defineEmits<{
  /** The stand-in at the top was chosen. */
  standIn: []
  /** One change of the model settings, naming only the parts it changes. */
  change: [change: ModelSettingsChange]
  /** The user's context limit on the selected model, for all their conversations with it. */
  contextLimit: [change: ContextLimitChange]
}>()

const rows = computed(() => modelMenuRows(props.providers, props.models, props.settings, props.standIn !== undefined))
const context = computed(() => (props.withoutContext ? null : rows.value.context))
const fast = computed(() => (props.withoutFast ? null : rows.value.fast))

function setFast(enabled: boolean) {
  const tier = fast.value?.tier
  if (!tier)
    return
  emit('change', { serviceTierId: enabled ? tier.id : null })
}

function setEffort(next: string) {
  emit('change', { thinkingEffort: next })
}

function setContextLimit(tokens: number | null) {
  const current = rows.value.selected
  if (!current || current.model.contextLimit === tokens)
    return
  emit('contextLimit', { providerId: current.providerId, modelId: current.modelId, tokens })
}

// A switch is one change: the menu decides what of the settings the new model keeps.
function selectModel(providerId: string, model: ModelInfo, selected: boolean) {
  if (selected)
    return
  emit('change', modelSwitch(props.settings, rows.value.selected?.model, providerId, model))
}

function selectStandIn() {
  if (!rows.value.standIn?.selected)
    emit('standIn')
}
</script>

<template>
  <Menu>
    <template v-if="standIn && rows.standIn">
      <MenuItem
        :icon="standIn.icon"
        :label="standIn.label"
        :detail="standIn.detail"
        choice
        :is-selected="rows.standIn.selected"
        @select="selectStandIn"
      />
      <MenuDivider />
    </template>
    <MenuItem
      v-if="fast"
      :icon="Zap"
      :label="t('models.fast')"
      @select="setFast(!fast.on)"
    >
      <template #suffix>
        <Switch
          :model-value="fast.on"
          size="sm"
          @click.stop
          @update:model-value="setFast"
        />
      </template>
    </MenuItem>
    <MenuItem
      v-if="rows.reasoning"
      :icon="Brain"
      :label="t('models.reasoning')"
      :value="rows.reasoning.value"
    >
      <template #submenu>
        <Menu>
          <MenuItem
            v-for="option in rows.reasoning.options"
            :key="option.label"
            :icon="option.icon"
            :label="option.label"
            choice
            :is-selected="option.selected"
            @select="setEffort(option.effort)"
          />
        </Menu>
      </template>
    </MenuItem>
    <MenuItem
      v-if="context"
      :icon="Layers"
      :label="t('models.context')"
      :value="context.value"
    >
      <template #submenu>
        <ContextMenu
          :options="context.options"
          @select="setContextLimit(context.options[$event].tokens)"
        />
      </template>
    </MenuItem>
    <MenuDivider v-if="fast || rows.reasoning || context" />
    <MenuItem :icon="Cpu" :label="t('models.model')" :value="rows.model.value">
      <template #submenu>
        <Menu>
          <MenuGroup
            v-for="group in rows.model.groups"
            :key="group.provider.id"
            :label="group.provider.label"
          >
            <MenuItem
              v-for="item in group.models"
              :key="`${group.provider.id}:${item.model.id}`"
              :label="item.model.name"
              choice
              :is-selected="item.selected"
              :disabled="group.unchoosable !== null"
              :disabled-reason="group.unchoosable ?? undefined"
              @select="selectModel(group.provider.id, item.model, item.selected)"
            />
          </MenuGroup>
        </Menu>
      </template>
    </MenuItem>
  </Menu>
</template>
