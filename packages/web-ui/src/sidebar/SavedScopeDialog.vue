<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import type { Action } from '../ui/operations'
import SettingsRow from '../settings/SettingsRow.vue'
import { conversationTitle } from '../agent/conversation-title'
import { projectTarget } from './project-hosts'

/**
 * The question before a project that keeps one list of saved messages goes back to a list per
 * conversation: a message is in one list, so the project's go to the one conversation the pop-up
 * names, and every other conversation starts with an empty list.
 *
 * The pop-up lists the project's conversations that are not archived, the latest changed first, and
 * starts on `initial`. Move shows the switch while it runs, as the project's row does.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The project's id, the target of the switch. */
  id: string
  /** How many messages the project's list keeps. */
  count: number
  /** The project's conversations that are not archived, the latest changed first. */
  conversations: readonly { id: string; title: string | null }[]
  /** The conversation the pop-up starts on. */
  initial: string | null
  /** Switches, the project's messages going to the conversation `to`; Move works until it settles. */
  onMove?: Action<[to: string]>
}>()

const emit = defineEmits<{
  close: []
}>()

const to = ref<string | null>(null)
watch(
  () => [props.isOpen, props.initial] as const,
  ([open]) => {
    if (open) {
      to.value = props.initial ?? props.conversations[0]?.id ?? null
    }
  },
  { immediate: true },
)

const chosen = computed(() => props.conversations.find((conversation) => conversation.id === to.value) ?? null)

function move(): unknown {
  if (to.value) {
    return props.onMove?.(to.value)
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="t('savedScope.title')" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('savedScope.title') }}</h3>
        <p class="mt-0.5 text-explanation leading-5 text-fg-muted">
          {{ t('savedScope.body', { count }, count) }}
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow :label="t('savedScope.to')">
          <Dropdown
            :overlay-store="overlayStore"
            variant="field"
            size="sm"
            width="shrink"
            :trigger-label="t('savedScope.to')"
          >
            <template #trigger>
              <span class="min-w-0 truncate">{{ chosen ? conversationTitle(chosen.title) : '' }}</span>
            </template>
            <template #content="{ close }">
              <Menu class="max-h-72">
                <MenuItem
                  v-for="conversation in conversations"
                  :key="conversation.id"
                  :label="conversationTitle(conversation.title)"
                  choice
                  :is-selected="conversation.id === to"
                  @select="close(); to = conversation.id"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
      </div>
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button
        variant="primary"
        :disabled="!to"
        :operation="{ action: 'change', target: projectTarget(id) }"
        @click="move"
      >
        {{ t('savedScope.action') }}
      </Button>
    </template>
  </Dialog>
</template>
