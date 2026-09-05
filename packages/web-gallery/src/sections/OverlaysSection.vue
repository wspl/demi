<script setup lang="ts">
import { Cloud, Copy, Link, Monitor, Pencil, Plus, Trash2, Unlink } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Toast from '@demicodes/web-ui/ui/Toast.vue'
import ContextMenu from '@demicodes/web-ui/ui/ContextMenu.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import DropdownTrigger from '@demicodes/web-ui/ui/DropdownTrigger.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ref } from 'vue'
import HostPicker from '@demicodes/web-ui/hosts/HostPicker.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'

const items = [
  { id: 'neutral', label: 'Neutral' },
  { id: 'hairline', label: 'Hairline' },
  { id: 'carved', label: 'Carved' },
  { id: 'overlay', label: 'Overlay' },
]
const tallActions = Array.from({ length: 24 }, (_, i) => `Action ${String(i + 1).padStart(2, '0')}`)
const tallOptions = Array.from({ length: 24 }, (_, i) => ({
  id: `opt-${i + 1}`,
  label: `Option ${String(i + 1).padStart(2, '0')}`,
}))
const dialogOpen = ref(false)
const inlineDialogOpen = ref(true)
const pinDangerToast = ref(true)
const pinCopiedToast = ref(true)

const paradigmSelected = ref('hairline')
const densitySelected = ref('compact')
const choiceIconSelected = ref('hairline')
const choiceIconFocused = ref('carved')
const filterMenuSelected = ref('opt-4')
const virtualMenuSelected = ref('opt-8')
const dropdownChoiceSelected = ref('hairline')
const dropdownEffortSelected = ref('medium')
const dropdownInlineSelected = ref('hairline')
const dropdownFilterSelected = ref('hairline')
const submenuModel = ref('sonnet')
const submenuFast = ref(false)
const submenuReasoning = ref(2)
const submenuReasoningLabels = ['Off', 'Low', 'Medium', 'High'] as const
const triggerDefaultClosed = ref(false)
const triggerDefaultOpen = ref(true)
const triggerGhostClosed = ref(false)
const triggerGhostOpen = ref(true)
const triggerSmClosed = ref(false)

const effortItems = [
  { id: 'low', label: 'Low' },
  { id: 'medium', label: 'Medium' },
  { id: 'high', label: 'High' },
]
const submenuModels = [
  { id: 'sonnet', label: 'Claude Sonnet' },
  { id: 'opus', label: 'Claude Opus' },
  { id: 'gpt', label: 'GPT-5' },
]

function itemLabel(id: string, list: { id: string; label: string }[] = items) {
  return list.find(item => item.id === id)?.label ?? id
}
</script>

<template>
  <div class="space-y-8">
    <GallerySection title="Tooltip" note="Hover, placement, rich overlay, and suppressed.">
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="hover">
          <Tooltip content="Send the current turn">
            <Button size="md">Hover me</Button>
          </Tooltip>
        </GallerySpecimen>
        <GallerySpecimen variant="top">
          <Tooltip content="Send the current turn">
            <Button size="md">Top</Button>
          </Tooltip>
        </GallerySpecimen>
        <GallerySpecimen variant="bottom">
          <Tooltip content="Model and reasoning" placement="bottom">
            <Button size="md">Below</Button>
          </Tooltip>
        </GallerySpecimen>
        <GallerySpecimen variant="overlay">
          <Tooltip placement="right">
            <Button size="md">Rich</Button>
            <template #overlay>
              <div class="text-[12px] leading-4">
                <div class="text-fg">34% used <span class="text-fg-subtle">(61.2K / 180K)</span></div>
                <div class="mt-1 text-fg-subtle">Click to compact</div>
              </div>
            </template>
          </Tooltip>
        </GallerySpecimen>
        <GallerySpecimen variant="disabled">
          <Tooltip content="Never shows" disabled>
            <Button size="md" disabled>Suppressed</Button>
          </Tooltip>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="Menu" note="Actions, choices, submenus, tall, and filter.">
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="actions">
          <Menu>
            <MenuItem :icon="Pencil" label="Rename" shortcut="↵" />
            <MenuItem :icon="Copy" label="Duplicate" shortcut="⌘D" />
            <MenuDivider />
            <MenuItem label="Delete" :icon="Trash2" is-danger />
            <MenuItem label="Disabled" disabled />
            <MenuItem label="Running" disabled disabled-reason="The session is still running" />
          </Menu>
        </GallerySpecimen>
        <GallerySpecimen variant="choices">
          <Menu iconless>
            <MenuGroup label="Paradigm">
              <MenuItem label="Neutral" choice :is-selected="paradigmSelected === 'neutral'" @select="paradigmSelected = 'neutral'" />
              <MenuItem label="Hairline" choice :is-selected="paradigmSelected === 'hairline'" @select="paradigmSelected = 'hairline'" />
              <MenuItem label="Carved" choice :is-selected="paradigmSelected === 'carved'" @select="paradigmSelected = 'carved'" />
              <MenuItem label="Overlay" choice :is-selected="paradigmSelected === 'overlay'" @select="paradigmSelected = 'overlay'" />
            </MenuGroup>
            <MenuGroup label="Density">
              <MenuItem label="Compact" choice :is-selected="densitySelected === 'compact'" @select="densitySelected = 'compact'" />
              <MenuItem label="Regular" choice :is-selected="densitySelected === 'regular'" @select="densitySelected = 'regular'" />
            </MenuGroup>
          </Menu>
        </GallerySpecimen>
        <GallerySpecimen variant="choice · icon · focus">
          <Menu>
            <MenuItem :icon="Pencil" label="Hairline" choice :is-selected="choiceIconSelected === 'hairline'" :is-focused="choiceIconFocused === 'hairline'" @select="choiceIconSelected = 'hairline'; choiceIconFocused = 'hairline'" />
            <MenuItem :icon="Pencil" label="Carved" choice :is-selected="choiceIconSelected === 'carved'" :is-focused="choiceIconFocused === 'carved'" @select="choiceIconSelected = 'carved'; choiceIconFocused = 'carved'" />
            <MenuItem :icon="Pencil" label="Overlay" choice :is-selected="choiceIconSelected === 'overlay'" :is-focused="choiceIconFocused === 'overlay'" @select="choiceIconSelected = 'overlay'; choiceIconFocused = 'overlay'" />
          </Menu>
        </GallerySpecimen>
      </div>
      <GalleryOverlayWell size="wide">
        <GallerySpecimen variant="iconless · submenu">
          <Menu iconless>
            <MenuItem label="Fast Mode" @select="submenuFast = !submenuFast">
              <template #suffix>
                <Switch v-model="submenuFast" size="sm" @click.stop />
              </template>
            </MenuItem>
            <MenuItem submenu-open label="Reasoning" :value="submenuReasoningLabels[submenuReasoning]">
              <template #submenu>
                <Menu iconless>
                  <MenuItem
                    v-for="(label, index) in submenuReasoningLabels"
                    :key="label"
                    :label="label" choice
                    :is-selected="submenuReasoning === index"
                    @select="submenuReasoning = index"
                  />
                </Menu>
              </template>
            </MenuItem>
            <MenuItem label="Model" :value="itemLabel(submenuModel, submenuModels)">
              <template #submenu>
                <Menu iconless>
                  <MenuItem label="Claude Sonnet" choice :is-selected="submenuModel === 'sonnet'" @select="submenuModel = 'sonnet'" />
                  <MenuItem label="Claude Opus" choice :is-selected="submenuModel === 'opus'" @select="submenuModel = 'opus'" />
                  <MenuItem label="GPT-5" choice :is-selected="submenuModel === 'gpt'" @select="submenuModel = 'gpt'" />
                </Menu>
              </template>
            </MenuItem>
          </Menu>
        </GallerySpecimen>
      </GalleryOverlayWell>
      <GallerySpecimen variant="host menu · label/value and status">
        <Menu>
          <MenuItem :icon="Monitor" label="Main host" value="zan-mbp" has-submenu>
            <template #submenu>
              <HostPicker
                :devices="[{ id: 'mac', name: 'zan-mbp', online: true }, { id: 'build', name: 'build-01', online: true }]"
                include-cloud require-online selected-id="mac"
              />
            </template>
          </MenuItem>
          <MenuGroup label="Attached hosts">
            <MenuItem label="build-01" indicator="success" indicator-label="Online" has-submenu>
              <template #submenu>
                <Menu>
                  <MenuItem :icon="Monitor" label="Use as main environment…" />
                  <MenuItem :icon="Unlink" label="Detach" />
                </Menu>
              </template>
            </MenuItem>
          </MenuGroup>
          <MenuDivider />
          <MenuItem :icon="Plus" label="Attach device…" has-submenu>
            <template #submenu>
              <HostPicker
                :devices="[{ id: 'mac', name: 'zan-mbp', online: true }, { id: 'build', name: 'build-01', online: true }, { id: 'studio', name: 'studio', online: false }]"
                :bound-ids="['mac', 'build']"
              />
            </template>
          </MenuItem>
          <MenuItem :icon="Link" label="Connect new device…" />
        </Menu>
      </GallerySpecimen>
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="tall">
          <Menu iconless>
            <MenuItem v-for="label in tallActions" :key="label" :label="label" />
          </Menu>
        </GallerySpecimen>
        <GallerySpecimen variant="tall · filter">
          <Menu
            filterable
            :autofocus="false"
            filter-placeholder="Filter options"
            :items="tallOptions"
            :selected-id="filterMenuSelected"
            @select="filterMenuSelected = $event"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="virtual">
          <Menu
            :items="tallOptions"
            :selected-id="virtualMenuSelected"
            :item-height="28"
            :autofocus="false"
            @select="virtualMenuSelected = $event"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="filter · empty">
          <Menu
            filterable
            :autofocus="false"
            filter-placeholder="Filter options"
            empty-text="No items found"
            :items="tallOptions"
            initial-query="zzz"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="empty list">
          <Menu
            filterable
            :autofocus="false"
            filter-placeholder="Search conversations"
            empty-text="No conversations"
            :items="[]"
          />
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="DropdownTrigger" note="Open and close.">
      <div class="specimen-row">
        <GallerySpecimen variant="default · closed">
          <DropdownTrigger :is-open="triggerDefaultClosed" @click="triggerDefaultClosed = !triggerDefaultClosed">Menu</DropdownTrigger>
        </GallerySpecimen>
        <GallerySpecimen variant="default · open">
          <DropdownTrigger :is-open="triggerDefaultOpen" @click="triggerDefaultOpen = !triggerDefaultOpen">Menu</DropdownTrigger>
        </GallerySpecimen>
        <GallerySpecimen variant="ghost · closed">
          <DropdownTrigger variant="ghost" :is-open="triggerGhostClosed" @click="triggerGhostClosed = !triggerGhostClosed">claude-sonnet</DropdownTrigger>
        </GallerySpecimen>
        <GallerySpecimen variant="ghost · open">
          <DropdownTrigger variant="ghost" :is-open="triggerGhostOpen" @click="triggerGhostOpen = !triggerGhostOpen">claude-sonnet</DropdownTrigger>
        </GallerySpecimen>
        <GallerySpecimen variant="sm · closed">
          <DropdownTrigger size="sm" :is-open="triggerSmClosed" @click="triggerSmClosed = !triggerSmClosed">Menu</DropdownTrigger>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="Dropdown" note="Trigger plus slotted Menu.">
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="default · md · actions">
          <Dropdown variant="default" :overlay-store="appOverlayStore">
            <template #trigger>Menu</template>
            <template #content="{ close }">
              <Menu @click="close">
                <MenuItem label="Rename" shortcut="↵" />
                <MenuItem label="Duplicate" shortcut="⌘D" />
                <MenuDivider />
                <MenuItem label="Delete" :icon="Trash2" is-danger />
              </Menu>
            </template>
          </Dropdown>
        </GallerySpecimen>
        <GallerySpecimen variant="ghost · md · choice">
          <Dropdown variant="ghost" :overlay-store="appOverlayStore">
            <template #trigger>claude-sonnet</template>
            <template #content="{ close }">
              <Menu iconless>
                <MenuItem
                  v-for="item in items"
                  :key="item.id"
                  :label="item.label" choice
                  :is-selected="item.id === dropdownChoiceSelected"
                  @select="dropdownChoiceSelected = item.id; close()"
                />
              </Menu>
            </template>
          </Dropdown>
        </GallerySpecimen>
        <GallerySpecimen variant="ghost · sm">
          <Dropdown variant="ghost" size="sm" :overlay-store="appOverlayStore">
            <template #trigger>{{ itemLabel(dropdownEffortSelected, effortItems) }}</template>
            <template #content="{ close }">
              <Menu iconless>
                <MenuItem
                  v-for="item in effortItems"
                  :key="item.id"
                  :label="item.label" choice
                  :is-selected="item.id === dropdownEffortSelected"
                  @select="dropdownEffortSelected = item.id; close()"
                />
              </Menu>
            </template>
          </Dropdown>
        </GallerySpecimen>
        <GallerySpecimen variant="default · sm">
          <Dropdown variant="default" size="sm" :overlay-store="appOverlayStore">
            <template #trigger>Menu</template>
            <template #content="{ close }">
              <Menu iconless @click="close">
                <MenuItem label="Rename" shortcut="↵" />
                <MenuItem label="Duplicate" shortcut="⌘D" />
              </Menu>
            </template>
          </Dropdown>
        </GallerySpecimen>
        <GallerySpecimen variant="custom trigger">
          <Dropdown :overlay-store="appOverlayStore">
            <template #trigger="{ isOpen }">
              <IconButton :icon="Plus" variant="ghost" circle :pressed="isOpen" />
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem :icon="Plus" label="Attach files" @select="close()" />
              </Menu>
            </template>
          </Dropdown>
        </GallerySpecimen>
        <GallerySpecimen variant="filter">
          <Dropdown variant="default" :overlay-store="appOverlayStore">
            <template #trigger>{{ itemLabel(dropdownFilterSelected) }}</template>
            <template #content="{ close }">
              <Menu
                filterable
                filter-placeholder="Filter paradigms"
                :items="items"
                :selected-id="dropdownFilterSelected"
                @select="dropdownFilterSelected = $event; close()"
              />
            </template>
          </Dropdown>
        </GallerySpecimen>
      </div>
      <GalleryOverlayWell>
        <GallerySpecimen variant="pinned open">
          <Dropdown variant="ghost" :overlay-store="appOverlayStore" open>
            <template #trigger>{{ itemLabel(dropdownInlineSelected) }}</template>
            <template #content>
              <Menu iconless>
                <MenuItem
                  v-for="item in items"
                  :key="item.id"
                  :label="item.label" choice
                  :is-selected="item.id === dropdownInlineSelected"
                  @select="dropdownInlineSelected = item.id"
                />
              </Menu>
            </template>
          </Dropdown>
        </GallerySpecimen>
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="ContextMenu" note="Right-click Menu.">
      <GallerySpecimen variant="right-click">
        <ContextMenu :overlay-store="appOverlayStore">
          <template #trigger>
            <div class="flex h-24 w-72 items-center justify-center rounded-lg bg-surface-raised text-[13px] text-fg-muted ring-1 ring-line">
              Right-click this surface
            </div>
          </template>
          <template #menu>
            <MenuItem :icon="Pencil" label="Rename" shortcut="↵" />
            <MenuItem label="New tab" />
            <MenuDivider />
            <MenuItem :icon="Trash2" label="Close" is-danger />
          </template>
        </ContextMenu>
      </GallerySpecimen>
    </GallerySection>

    <GallerySection title="Toast" note="Danger, Copied, and live host.">
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="danger">
          <div class="w-80">
            <Toast
              v-if="pinDangerToast"
              title="Failed to send message"
              message="WebSocket is closed"
              tone="danger"
              @dismiss="pinDangerToast = false"
            />
            <Button v-else size="md" @click="pinDangerToast = true">Show</Button>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="copied">
          <div class="w-80">
            <Toast v-if="pinCopiedToast" title="Copied" @dismiss="pinCopiedToast = false" />
            <Button v-else size="md" variant="ghost" @click="pinCopiedToast = true">Show</Button>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="live">
          <div class="flex flex-wrap gap-2">
            <Button
              size="md"
              @click="showToast({ title: 'Failed to send message', message: 'WebSocket is closed', tone: 'danger' })"
            >Fail send</Button>
            <Button size="md" variant="ghost" @click="showToast({ title: 'Copied' })">Copy id</Button>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="Dialog" note="Modal confirm. The pinned one starts open.">
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="open">
          <Button size="md" @click="dialogOpen = true">Open</Button>
        </GallerySpecimen>
      </div>
      <GalleryOverlayWell size="lg">
        <GallerySpecimen variant="pinned">
        <Button v-if="!inlineDialogOpen" size="md" @click="inlineDialogOpen = true">Open</Button>
        <Dialog :is-open="inlineDialogOpen" :overlay-store="appOverlayStore" @close="inlineDialogOpen = false">
          <div class="space-y-3 p-4">
            <h3 class="text-[15px] font-medium text-fg-emphasis">Keep this queued follow-up?</h3>
            <p class="text-[13px] leading-5 text-fg-muted">
              The expired-cookie case can wait. Keep the queued message for the next turn?
            </p>
            <div class="flex justify-end gap-2">
              <Button size="md" variant="ghost" @click="inlineDialogOpen = false">Cancel</Button>
              <Button size="md" variant="primary" @click="inlineDialogOpen = false">Keep</Button>
            </div>
          </div>
        </Dialog>
        </GallerySpecimen>
      </GalleryOverlayWell>
      <Dialog
        :is-open="dialogOpen"
        :overlay-store="appOverlayStore"
        @close="dialogOpen = false"
      >
        <div class="space-y-3 p-4">
          <h3 class="text-[15px] font-medium text-fg-emphasis">Keep this queued follow-up?</h3>
          <p class="text-[13px] leading-5 text-fg-muted">
            The expired-cookie case can wait. Keep the queued message for the next turn?
          </p>
          <div class="flex justify-end gap-2">
            <Button size="md" variant="ghost" @click="dialogOpen = false">Cancel</Button>
            <Button size="md" variant="primary" @click="dialogOpen = false">Keep</Button>
          </div>
        </div>
      </Dialog>
    </GallerySection>
  </div>
</template>
