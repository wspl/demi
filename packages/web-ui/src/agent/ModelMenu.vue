<script setup lang="ts">
import { computed } from 'vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import type { ContextLimitChange } from './context-limit'
import { modelSwitch, type ModelSettings, type ModelSettingsChange } from './model-selection'
import { modelMenuRows } from './model-menu'
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
   * Offers Parent's model at the top, chosen while `settings` is none: for
   * a subagent profile, whose children may infer with their parent's model.
   */
  parentModel?: boolean
}>()

const emit = defineEmits<{
  /** Parent's model was chosen. */
  parentModel: []
  /** One change of the model settings, naming only the parts it changes. */
  change: [change: ModelSettingsChange]
  /** The user's context limit on the selected model, for all their conversations with it. */
  contextLimit: [change: ContextLimitChange]
}>()

const rows = computed(() => modelMenuRows(props.providers, props.models, props.settings, props.parentModel === true))

function setFast(enabled: boolean) {
  const fast = rows.value.fast
  if (!fast)
    return
  emit('change', { serviceTierId: enabled ? fast.tier.id : null })
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

function selectParent() {
  if (!rows.value.parent?.selected)
    emit('parentModel')
}
</script>

<template>
  <Menu iconless>
    <template v-if="rows.parent">
      <MenuItem
        label="Parent’s Model"
        choice
        :is-selected="rows.parent.selected"
        @select="selectParent"
      />
      <MenuDivider />
    </template>
    <MenuItem
      v-if="rows.fast"
      label="Fast Mode"
      @select="setFast(!rows.fast.on)"
    >
      <template #suffix>
        <Switch
          :model-value="rows.fast.on"
          size="sm"
          @click.stop
          @update:model-value="setFast"
        />
      </template>
    </MenuItem>
    <MenuItem
      v-if="rows.reasoning"
      label="Reasoning"
      :value="rows.reasoning.value"
    >
      <template #submenu>
        <Menu iconless>
          <MenuItem
            v-for="option in rows.reasoning.options"
            :key="option.label"
            :label="option.label"
            choice
            :is-selected="option.selected"
            @select="setEffort(option.effort)"
          />
        </Menu>
      </template>
    </MenuItem>
    <MenuItem
      v-if="rows.context"
      label="Context"
      :value="rows.context.value"
    >
      <template #submenu>
        <Menu iconless>
          <MenuItem
            v-for="option in rows.context.options"
            :key="option.window"
            :label="option.label"
            choice
            :is-selected="option.selected"
            @select="setContextLimit(option.tokens)"
          />
        </Menu>
      </template>
    </MenuItem>
    <MenuDivider v-if="rows.fast || rows.reasoning || rows.context" />
    <MenuItem label="Model" :value="rows.model.value">
      <template #submenu>
        <Menu iconless>
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
