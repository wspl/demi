<script setup lang="ts">
import { t } from './locales/text'
import { Folder, FolderOpen, SquarePen } from '@lucide/vue'
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import RowSpinner from '@demicodes/web-ui/ui/RowSpinner.vue'
import { provideRowTarget } from '@demicodes/web-ui/ui/operations'
import TruncatedText from '@demicodes/web-ui/ui/TruncatedText.vue'
import { ICON_PX, useIconPx } from '@demicodes/web-ui/ui/icon-metrics'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import DeviceMark from './DeviceMark.vue'
import { projectTarget, type ProjectQualifier } from './project-hosts'
import type { SidebarProject } from './types'

/** A project row: the checkout's name, the host it is on as the row's end mark, and the fold. */
const props = defineProps<{
  project: SidebarProject
  /**
   * Another project has its name: what tells this one apart, its host or the end of its parent
   * folders, in secondary text after the name, and where it is, the name's tooltip
   * (`projectQualifiers`).
   */
  qualifier?: ProjectQualifier
  collapsed: boolean
  /** The keyboard cursor is here. */
  focused: boolean
}>()

// An operation on the project, such as Remove Project from its menu, shows the spinner where the host's mark stands.
const working = provideRowTarget(() => projectTarget(props.project.id))

const phone = usePhone()
const icons = useIconPx()

const emit = defineEmits<{
  create: []
  toggle: []
}>()
</script>

<template>
  <div
    role="button"
    :aria-expanded="!collapsed"
    class="group/project relative flex h-7 cursor-default select-none items-center gap-2 rounded-md pl-2.5 pr-1.5 text-chrome text-fg transition-colors duration-200 ease-out phone:h-11 phone:gap-3 phone:rounded-lg phone:pl-3 phone:pr-0.5 phone:text-[16px]"
    :class="[focused ? 'focus-ring-shown' : '']"
    @click="emit('toggle')"
  >
    <component
      :is="collapsed ? Folder : FolderOpen"
      :size="icons.row"
      class="shrink-0 text-fg-muted"
    />
    <!-- The text, then the host's mark in its own box at the row's end, 6px from it, as a
         conversation's row ends. The mark stays on hover; New conversation stands to its left, its
         glyph 6px from the mark's as Archive's is, over the end of the text, which gives it the
         place by the same margin a conversation's title does. -->
    <span
      class="flex min-w-0 flex-1 items-center transition-[margin] duration-[80ms] ease-out group-hover/project:mr-6 group-focus-within/project:mr-6 phone:mr-8"
    >
      <!-- The project's name takes what the qualifier leaves; the qualifier, shown only beside a
           namesake and only while no button shows, takes its own width,
           at most half of the text's, and each is cut at its end. The hover chevron and the gaps around it (4px, 12px, 4px)
           are not text, so the device's half leaves out half of their 20px. -->
      <span class="flex min-w-0 flex-1 items-center gap-1">
        <!-- A namesake's name says where it is in its tooltip, since its qualifier gives its place to
             the buttons while the pointer is over the row. -->
        <Tooltip
          :content="qualifier?.hint"
          :disabled="!qualifier"
          class="flex min-w-0 flex-1 items-center gap-1"
        >
          <TruncatedText class="font-medium text-fg-emphasis" :text="project.name" />
          <FoldChevron
            :open="!collapsed"
            :size="ICON_PX.in20"
            class="text-fg-subtle opacity-0 group-hover/project:opacity-100 phone:opacity-100"
          />
        </Tooltip>
        <!-- The name stands only where the row shows no button: it gives its place to New
             conversation while the pointer is over the row, and a phone row, whose button always
             shows, has none. -->
        <span
          v-if="qualifier"
          class="flex min-w-0 max-w-[calc(50%-10px)] flex-[0_1_auto] group-hover/project:hidden group-focus-within/project:hidden phone:hidden"
        >
          <TruncatedText class="text-[11px] text-fg-subtle" :text="qualifier.text" />
        </span>
      </span>
    </span>
    <!-- The spinner takes the device mark's box, so the name keeps its width. -->
    <RowSpinner v-if="working" class="h-6! w-4! phone:size-8!" />
    <DeviceMark v-else :host="project.hostKind === 'cloud' ? { kind: 'cloud' } : project.device" />
    <span
      class="absolute inset-y-0 right-[23px] flex items-center opacity-0 pointer-events-none transition-opacity duration-150 motion-reduce:transition-none group-hover/project:opacity-100 group-hover/project:pointer-events-auto focus-within:opacity-100 focus-within:pointer-events-auto phone:right-9 phone:opacity-100 phone:pointer-events-auto"
    >
      <Tooltip :content="t('project.newConversation')" class="flex items-center">
        <IconButton
          :icon="SquarePen"
          :size="phone ? 'lg' : 'sm'"
          variant="ghost"
          :aria-label="t('project.newConversation')"
          tabindex="0"
          @keydown.enter.stop.prevent="emit('create')"
          @keydown.space.stop.prevent="emit('create')"
          @click.stop="emit('create')"
        />
      </Tooltip>
    </span>
  </div>
</template>
