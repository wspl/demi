<script setup lang="ts">
import { Copy, Folder, Pencil, Plus, Settings, Trash2 } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { showArchived } from '@demicodes/web-ui/sidebar/archived-toast'
import { productWould } from '../product-would'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Toast from '@demicodes/web-ui/ui/Toast.vue'
import UpdateFailedScreen from '@demicodes/web-ui/ui/UpdateFailedScreen.vue'
import ConnectionBanner from '@demicodes/web-ui/ui/ConnectionBanner.vue'
import ContextMenu from '@demicodes/web-ui/ui/ContextMenu.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import DropdownTrigger from '@demicodes/web-ui/ui/DropdownTrigger.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { reactive, ref } from 'vue'
import HostPicker from '@demicodes/web-ui/hosts/HostPicker.vue'
import AutofocusScope from '@demicodes/web-ui/ui/AutofocusScope.vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import { demoImageUrl } from '../fixtures/blocks'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { useGalleryView } from '../gallery-views'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE } from '@demicodes/web-ui/devices/state'

const { view } = useGalleryView()

const hostDevices: HostDeviceOption[] = [
  { id: 'mac', name: 'zan-mbp', state: 'online' },
  { id: 'build', name: 'build-01', state: 'updating' },
  { id: 'studio', name: 'studio', state: 'offline' },
]
const hostStatusItems = hostDevices.map((device) => ({
  id: device.id,
  label: device.name,
  indicator: DEVICE_STATE_TONE[device.state],
  indicatorLabel: DEVICE_STATE_LABEL[device.state],
}))
const primaryHost = ref<HostMenuHost>({
  id: 'mac',
  name: 'zan-mbp',
  kind: 'device',
  state: 'online',
})
const attachedHosts = ref<HostMenuHost[]>([
  ...hostDevices.filter(device => device.id !== 'mac').map(device => ({ ...device, kind: 'device' as const })),
  { id: 'managed-device', name: 'Cloud', kind: 'cloud', state: 'offline' },
])

function switchPrimaryHost(id: string) {
  const device = hostDevices.find(device => device.id === id)
  primaryHost.value = device
    ? { id: device.id, name: device.name, kind: 'device', state: device.state }
    : { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online' }
}

function attachHost(id: string) {
  const device = hostDevices.find(device => device.id === id)
  if (device)
    attachedHosts.value.push({ ...device, kind: 'device' })
}

function detachHost(id: string) {
  attachedHosts.value = attachedHosts.value.filter(device => device.id !== id)
}

/** A host's name as the host menu shows it; the Cloud is the one that is no device. */
function deviceName(id: string): string {
  return hostDevices.find(device => device.id === id)?.name ?? 'Cloud'
}
const statusSelected = ref('mac')

/** A menu row a specimen offers: its id and its label. */
interface MenuChoice {
  id: string
  label: TitleText
}

const items: MenuChoice[] = [
  { id: 'neutral', label: 'Neutral' },
  { id: 'hairline', label: 'Hairline' },
  { id: 'carved', label: 'Carved' },
  { id: 'overlay', label: 'Overlay' },
]
const tallActions: TitleText[] = Array.from({ length: 24 }, (_, i) => `Action ${String(i + 1).padStart(2, '0')}`)
const tallOptions = Array.from({ length: 24 }, (_, i): MenuChoice => ({
  id: `opt-${i + 1}`,
  label: `Option ${String(i + 1).padStart(2, '0')}`,
}))
const dialogOpen = ref(false)
const inlineDialogOpen = ref(true)
// Layers: a dialog, the model menu opened from it, and that menu's submenu, all pinned open.
const layersDialogOpen = ref(true)
const layersMenuOpen = ref(true)
const layersSource = ref<'parent' | 'own'>('own')
const layersSources = [
  { value: 'parent', label: 'Parent’s' },
  { value: 'own', label: 'Own' },
] as const
const layersModel = ref('opus')
const layersReasoning = ref('low')
const pinDangerToast = ref(true)
const pinRejectedToast = ref(true)
const pinNeutralToast = ref(true)
const pinCopiedToast = ref(true)
const pinUndoToast = ref(true)
/** Undo closes the pinned toast, as an action does, and the product restores the conversation. */
function undoPinnedArchive(): void {
  pinUndoToast.value = false
  productWould('Restore the Conversation')
}

const paradigmSelected = ref('hairline')
const densitySelected = ref('compact')
const choiceIconSelected = ref('hairline')
const choiceIconFocused = ref('carved')
const filterMenuSelected = ref('opt-4')
const filterEmptySelected = ref<string | undefined>()
const virtualMenuSelected = ref('opt-8')
const columnsEffort = ref('medium')
const columnsModel = ref('sonnet')
const columnsLonger = ref(true)
// The directory each width specimen shows; its menu switches it, as the header's directory menu does.
const widthDirectories = ['a-rather-long-directory-name', 'demi']
const widthDirectory = reactive({
  content: widthDirectories[0]!,
  shrink: widthDirectories[0]!,
  fill: widthDirectories[0]!,
})
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

const effortItems: MenuChoice[] = [
  { id: 'low', label: 'Low' },
  { id: 'medium', label: 'Medium' },
  { id: 'high', label: 'High' },
]
const submenuModels: MenuChoice[] = [
  { id: 'sonnet', label: 'Claude Sonnet' },
  { id: 'opus', label: 'Claude Opus' },
  { id: 'gpt', label: 'GPT-5' },
]

function itemLabel(id: string, list: MenuChoice[] = items): TitleText {
  return list.find(item => item.id === id)?.label ?? id
}
</script>

<template>
  <div class="space-y-8">
    <template v-if="view === 'menus'">
      <GallerySection
        title="Tooltip"
        note="Hover, placement, rich overlay, a picture framed evenly, and suppressed. Copy: the verb alone for a control on the thing it acts on (Archive, Pin, Copy, Edit); the object only away from it (Add project, New folder); a reason is a full sentence."
      >
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen variant="hover">
            <Tooltip content="Send the current turn">
              <Button size="md" @click="productWould('Send the Current Turn')">Hover Me</Button>
            </Tooltip>
          </GallerySpecimen>
          <GallerySpecimen variant="top">
            <Tooltip content="Send the current turn">
              <Button size="md" @click="productWould('Send the Current Turn')">Top</Button>
            </Tooltip>
          </GallerySpecimen>
          <GallerySpecimen variant="bottom">
            <Tooltip content="Model and reasoning" placement="bottom">
              <Button size="md" @click="productWould('Open the Model and Reasoning Menu')">Below</Button>
            </Tooltip>
          </GallerySpecimen>
          <GallerySpecimen variant="overlay">
            <Tooltip placement="right">
              <Button size="md" @click="productWould('Open the Model Menu')">Rich</Button>
              <template #overlay>
                <div class="text-[12px] leading-4">
                  <div class="text-fg">Claude Sonnet <span class="text-fg-subtle">(200K context)</span></div>
                  <div class="mt-1 text-fg-subtle">Reasoning: medium</div>
                </div>
              </template>
            </Tooltip>
          </GallerySpecimen>
          <GallerySpecimen variant="picture">
            <Tooltip placement="right" picture>
              <Button size="md" @click="productWould('Open before.png')">Picture</Button>
              <template #overlay>
                <img
                  :src="demoImageUrl"
                  alt="before.png"
                  class="block max-h-48 max-w-full rounded object-contain"
                />
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

      <GallerySection
        title="Menu"
        note="Actions, choices, submenus, tall, and filter. Enabled choices use normal text; being unselected does not dim them. Disabled rows keep their status dot and explain why they cannot be chosen. Paired devices show online status. Cloud shows no connection or lifecycle status and remains selectable while asleep; operations wake it automatically. Keys: Up and Down Arrow move, typing a name jumps to it, Return chooses; Right Arrow or Return opens a submenu, Left Arrow or Escape closes it. A group’s first row scrolls into view with its heading. When a menu closes, the focus returns to its opener, or to what had it before when the opener takes no focus; a click on another control keeps it."
      >
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen variant="actions">
            <Menu :autofocus="false">
              <MenuItem
                :icon="Pencil"
                label="Rename"
                shortcut="↵"
                @select="productWould('Rename')"
              />
              <MenuItem
                :icon="Copy"
                label="Duplicate"
                shortcut="⌘D"
                @select="productWould('Duplicate')"
              />
              <MenuDivider />
              <MenuItem
                label="Delete"
                :icon="Trash2"
                is-danger
                @select="productWould('Delete')"
              />
              <MenuItem label="Disabled" disabled />
              <MenuItem
                label="Running"
                disabled
                disabled-reason="The session is still running"
              />
            </Menu>
          </GallerySpecimen>
          <GallerySpecimen variant="choices">
            <Menu :autofocus="false" iconless>
              <MenuGroup label="Paradigm">
                <MenuItem
                  label="Neutral"
                  choice
                  :is-selected="paradigmSelected === 'neutral'"
                  @select="paradigmSelected = 'neutral'"
                />
                <MenuItem
                  label="Hairline"
                  choice
                  :is-selected="paradigmSelected === 'hairline'"
                  @select="paradigmSelected = 'hairline'"
                />
                <MenuItem
                  label="Carved"
                  choice
                  :is-selected="paradigmSelected === 'carved'"
                  @select="paradigmSelected = 'carved'"
                />
                <MenuItem
                  label="Overlay"
                  choice
                  :is-selected="paradigmSelected === 'overlay'"
                  @select="paradigmSelected = 'overlay'"
                />
              </MenuGroup>
              <MenuGroup label="Density">
                <MenuItem
                  label="Compact"
                  choice
                  :is-selected="densitySelected === 'compact'"
                  @select="densitySelected = 'compact'"
                />
                <MenuItem
                  label="Regular"
                  choice
                  :is-selected="densitySelected === 'regular'"
                  @select="densitySelected = 'regular'"
                />
              </MenuGroup>
            </Menu>
          </GallerySpecimen>
          <GallerySpecimen variant="choice · icon · focus">
            <Menu :autofocus="false">
              <MenuItem
                :icon="Pencil"
                label="Hairline"
                choice
                :is-selected="choiceIconSelected === 'hairline'"
                :is-focused="choiceIconFocused === 'hairline'"
                @select="choiceIconSelected = 'hairline'; choiceIconFocused = 'hairline'"
              />
              <MenuItem
                :icon="Pencil"
                label="Carved"
                choice
                :is-selected="choiceIconSelected === 'carved'"
                :is-focused="choiceIconFocused === 'carved'"
                @select="choiceIconSelected = 'carved'; choiceIconFocused = 'carved'"
              />
              <MenuItem
                :icon="Pencil"
                label="Overlay"
                choice
                :is-selected="choiceIconSelected === 'overlay'"
                :is-focused="choiceIconFocused === 'overlay'"
                @select="choiceIconSelected = 'overlay'; choiceIconFocused = 'overlay'"
              />
            </Menu>
          </GallerySpecimen>
        </div>
        <GalleryOverlayWell size="wide">
          <GallerySpecimen variant="iconless · submenu">
            <Menu :autofocus="false" iconless>
              <MenuItem label="Fast Mode" @select="submenuFast = !submenuFast">
                <template #suffix>
                  <Switch
                    v-model="submenuFast"
                    size="sm"
                    @click.stop
                  />
                </template>
              </MenuItem>
              <MenuItem
                submenu-open
                label="Reasoning"
                :value="submenuReasoningLabels[submenuReasoning]"
              >
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
              <MenuItem
                label="Model"
                :value="itemLabel(submenuModel, submenuModels)"
              >
                <template #submenu>
                  <Menu iconless>
                    <MenuItem
                      label="Claude Sonnet"
                      choice
                      :is-selected="submenuModel === 'sonnet'"
                      @select="submenuModel = 'sonnet'"
                    />
                    <MenuItem
                      label="Claude Opus"
                      choice
                      :is-selected="submenuModel === 'opus'"
                      @select="submenuModel = 'opus'"
                    />
                    <MenuItem
                      label="GPT-5"
                      choice
                      :is-selected="submenuModel === 'gpt'"
                      @select="submenuModel = 'gpt'"
                    />
                  </Menu>
                </template>
              </MenuItem>
            </Menu>
          </GallerySpecimen>
        </GalleryOverlayWell>
        <GallerySpecimen variant="host picker · online, bound and offline">
          <!-- A picker shown open beside the others: it would take the keys as it does in its popover. -->
          <AutofocusScope :enabled="false">
            <HostPicker
              :devices="hostDevices"
              :bound-ids="['build']"
              @select="productWould(`Selected ${$event}`)"
              @connect="productWould('Add Device')"
            />
          </AutofocusScope>
        </GallerySpecimen>
        <GallerySpecimen variant="status dots · virtual list without icons">
          <Menu
            :autofocus="false"
            :items="hostStatusItems"
            :item-height="28"
            :selected-id="statusSelected"
            @select="statusSelected = $event"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="cloud host · no status dot">
          <HostMenu
            :primary-host="{ id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'offline' }"
            :attached-hosts="[]"
            :devices="hostDevices"
            @switch-primary="productWould(`Move the Conversation to ${deviceName($event)}`)"
            @attach="productWould(`Attach ${deviceName($event)}`)"
            @detach="productWould(`Detach ${deviceName($event)}`)"
            @connect="productWould('Add Device')"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="host menu · a name longer than the button, whole on hover">
          <HostMenu
            :primary-host="{ id: 'lab', name: 'lab-workstation-with-a-long-hostname', kind: 'device', state: 'online' }"
            :attached-hosts="[]"
            :devices="hostDevices"
            @switch-primary="productWould(`Move the Conversation to ${deviceName($event)}`)"
            @attach="productWould(`Attach ${deviceName($event)}`)"
            @detach="productWould(`Detach ${deviceName($event)}`)"
            @connect="productWould('Add Device')"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="host menu · label/value and status">
          <HostMenu
            :primary-host="primaryHost"
            :attached-hosts="attachedHosts"
            :devices="hostDevices"
            @switch-primary="switchPrimaryHost"
            @attach="attachHost"
            @detach="detachHost"
            @connect="productWould('Add Device')"
          />
        </GallerySpecimen>
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen variant="label/value · columns">
            <Menu :autofocus="false" iconless>
              <MenuItem
                label="Reasoning"
                :value="itemLabel(columnsEffort, effortItems)"
              >
                <template #submenu>
                  <Menu iconless>
                    <MenuItem
                      v-for="item in effortItems"
                      :key="item.id"
                      :label="item.label"
                      choice
                      :is-selected="item.id === columnsEffort"
                      @select="columnsEffort = item.id"
                    />
                  </Menu>
                </template>
              </MenuItem>
              <MenuItem
                label="Model"
                :value="itemLabel(columnsModel, submenuModels)"
              >
                <template #submenu>
                  <Menu iconless>
                    <MenuItem
                      v-for="item in submenuModels"
                      :key="item.id"
                      :label="item.label"
                      choice
                      :is-selected="item.id === columnsModel"
                      @select="columnsModel = item.id"
                    />
                  </Menu>
                </template>
              </MenuItem>
              <MenuItem
                label="A Much Longer Label"
                :value="columnsLonger ? 'On' : 'Off'"
                @select="columnsLonger = !columnsLonger"
              />
              <MenuItem
                label="Provider"
                value="Anthropic"
                @select="productWould('Open the Provider Settings')"
              />
            </Menu>
          </GallerySpecimen>
          <GallerySpecimen variant="shortcuts · columns">
            <Menu :autofocus="false" iconless>
              <MenuItem label="Rename" shortcut="↵" @select="productWould('Rename')" />
              <MenuItem
                label="Duplicate Conversation"
                shortcut="⌘D"
                @select="productWould('Duplicate Conversation')"
              />
              <MenuItem label="Pin" shortcut="⌘⇧P" @select="productWould('Pin')" />
              <MenuItem label="Archive" @select="productWould('Archive')" />
            </Menu>
          </GallerySpecimen>
        </div>
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen variant="tall">
            <Menu :autofocus="false" iconless>
              <MenuItem
                v-for="label in tallActions"
                :key="label"
                :label="label"
                @select="productWould(label)"
              />
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
              empty-text="No Items Found"
              :items="tallOptions"
              initial-query="zzz"
              :selected-id="filterEmptySelected"
              @select="filterEmptySelected = $event"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="empty list">
            <Menu
              filterable
              :autofocus="false"
              filter-placeholder="Search conversations"
              empty-text="No Conversations"
              :items="[]"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection title="DropdownTrigger" note="Open and close.">
        <div class="specimen-row">
          <GallerySpecimen variant="default · closed">
            <DropdownTrigger
              :is-open="triggerDefaultClosed"
              @click="triggerDefaultClosed = !triggerDefaultClosed"
            >Menu</DropdownTrigger>
          </GallerySpecimen>
          <GallerySpecimen variant="default · open">
            <DropdownTrigger
              :is-open="triggerDefaultOpen"
              @click="triggerDefaultOpen = !triggerDefaultOpen"
            >Menu</DropdownTrigger>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · closed">
            <DropdownTrigger
              variant="ghost"
              :is-open="triggerGhostClosed"
              @click="triggerGhostClosed = !triggerGhostClosed"
            >claude-sonnet</DropdownTrigger>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · open">
            <DropdownTrigger
              variant="ghost"
              :is-open="triggerGhostOpen"
              @click="triggerGhostOpen = !triggerGhostOpen"
            >claude-sonnet</DropdownTrigger>
          </GallerySpecimen>
          <GallerySpecimen variant="sm · closed">
            <DropdownTrigger
              size="sm"
              :is-open="triggerSmClosed"
              @click="triggerSmClosed = !triggerSmClosed"
            >Menu</DropdownTrigger>
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
                  <MenuItem label="Rename" shortcut="↵" @select="productWould('Rename')" />
                  <MenuItem label="Duplicate" shortcut="⌘D" @select="productWould('Duplicate')" />
                  <MenuDivider />
                  <MenuItem
                    label="Delete"
                    :icon="Trash2"
                    is-danger
                    @select="productWould('Delete')"
                  />
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
            <Dropdown
              variant="ghost"
              size="sm"
              :overlay-store="appOverlayStore"
            >
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
            <Dropdown
              variant="default"
              size="sm"
              :overlay-store="appOverlayStore"
            >
              <template #trigger>Menu</template>
              <template #content="{ close }">
                <Menu iconless @click="close">
                  <MenuItem label="Rename" shortcut="↵" @select="productWould('Rename')" />
                  <MenuItem label="Duplicate" shortcut="⌘D" @select="productWould('Duplicate')" />
                </Menu>
              </template>
            </Dropdown>
          </GallerySpecimen>
          <GallerySpecimen variant="custom trigger">
            <Dropdown :overlay-store="appOverlayStore">
              <template #trigger="{ isOpen }">
                <IconButton
                  :icon="Plus"
                  variant="ghost"
                  circle
                  :pressed="isOpen"
                />
              </template>
              <template #content="{ close }">
                <Menu @click="close">
                  <MenuItem
                    :icon="Plus"
                    label="Attach Files…"
                    @select="productWould('Attach Files')"
                  />
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
        <GalleryOverlayWell size="lg">
          <GallerySpecimen variant="pinned open">
            <Dropdown
              variant="ghost"
              :overlay-store="appOverlayStore"
              open
            >
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

      <GallerySection
        title="Dropdown Width"
        note="How wide a dropdown is, one of three widths: content keeps its trigger’s width whatever the row, so in a row too narrow for it what follows is pushed out; shrink keeps it while the row has room and gives way when it has not, its label truncating; fill spans the row and gives way the same. Every wrapper around the trigger gives way alike. Each frame resizes from its corner."
      >
        <GallerySpecimen v-for="width in (['content', 'shrink', 'fill'] as const)" :key="width" :variant="width" wide>
          <div class="flex max-w-full resize-x items-center gap-1 overflow-hidden pb-3" style="width: 20rem; min-width: 4rem">
            <Dropdown :overlay-store="appOverlayStore" :width="width" :variant="width === 'fill' ? 'field' : undefined" trigger-label="Directory">
              <!-- A field fills; the others are a button as wide as its label, the way the header's directory menu is. -->
              <template #trigger>
                <template v-if="width === 'fill'">
                  <Folder :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
                  <span class="min-w-0 flex-1 truncate">{{ widthDirectory[width] }}</span>
                </template>
                <Button v-else variant="ghost" class="max-w-full">
                  <Folder :size="ICON_PX.in28" />
                  <span class="truncate">{{ widthDirectory[width] }}</span>
                </Button>
              </template>
              <template #content="{ close }">
                <Menu @click="close">
                  <MenuItem
                    v-for="directory in widthDirectories"
                    :key="directory"
                    :icon="Folder"
                    :label="directory"
                    @select="widthDirectory[width] = directory"
                  />
                </Menu>
              </template>
            </Dropdown>
            <IconButton
              :icon="Settings"
              variant="ghost"
              aria-label="Settings"
              @click="productWould('Open Settings')"
            />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection title="ContextMenu" note="Right-click Menu.">
        <GallerySpecimen variant="right-click">
          <ContextMenu :overlay-store="appOverlayStore">
            <template #trigger>
              <div
                class="flex h-24 w-72 items-center justify-center rounded-lg bg-surface-raised text-[13px] text-fg-muted ring-1 ring-line"
              >
              Right-click this surface
              </div>
            </template>
            <template #menu>
              <MenuItem
                :icon="Pencil"
                label="Rename"
                shortcut="↵"
                @select="productWould('Rename')"
              />
              <MenuItem label="New Tab" @select="productWould('New Tab')" />
              <MenuDivider />
              <MenuItem
                :icon="Trash2"
                label="Close"
                is-danger
                @select="productWould('Close')"
              />
            </template>
          </ContextMenu>
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'dialogs'">
      <GallerySection
        title="Toast"
        note="Toasts stand in the top-right corner, below the bar atop the page, the newest nearest the corner, as macOS places its notifications: they never cover the composer or the actions at the bottom of a pane. The mark follows what happened: a cross for a request that failed, an i for a fact about one that neither worked nor failed, a check for an action that worked where nothing else shows it. A toast may offer one action, which closes it. A failure stays until it is closed; any other closes after six seconds, or ten when it offers an action such as Undo, as Gmail keeps its Undo, paused while the pointer is over the toasts. Danger, a rejected action whose title wraps, a fact, Copied, Undo, and live host."
      >
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen variant="danger">
            <div class="w-80">
              <Toast
                v-if="pinDangerToast"
                title="Failed to Send Message"
                message="WebSocket is closed"
                tone="danger"
                @dismiss="pinDangerToast = false"
              />
              <Button
                v-else
                size="md"
                @click="pinDangerToast = true"
              >Show</Button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="rejected">
            <div class="w-80">
              <Toast
                v-if="pinRejectedToast"
                title="Move the project’s conversations to another environment before removing it."
                tone="danger"
                @dismiss="pinRejectedToast = false"
              />
              <Button
                v-else
                size="md"
                @click="pinRejectedToast = true"
              >Show</Button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="neutral">
            <div class="w-80">
              <Toast
                v-if="pinNeutralToast"
                title="Not in This Workspace"
                message="The tree shows /Users/zan/.demi/acceptance-workspace only."
                tone="neutral"
                @dismiss="pinNeutralToast = false"
              />
              <Button
                v-else
                size="md"
                variant="ghost"
                @click="pinNeutralToast = true"
              >Show</Button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="copied">
            <div class="w-80">
              <Toast
                v-if="pinCopiedToast"
                title="Copied"
                tone="success"
                @dismiss="pinCopiedToast = false"
              />
              <Button
                v-else
                size="md"
                variant="ghost"
                @click="pinCopiedToast = true"
              >Show</Button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="undo">
            <div class="w-80">
              <Toast
                v-if="pinUndoToast"
                title="Conversation Archived"
                tone="success"
                action="Undo"
                @act="undoPinnedArchive"
                @dismiss="pinUndoToast = false"
              />
              <Button
                v-else
                size="md"
                variant="ghost"
                @click="pinUndoToast = true"
              >Show</Button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="live">
            <div class="flex flex-wrap gap-2">
              <Button
                size="md"
                @click="showToast({ title: 'Failed to Send Message', message: 'WebSocket is closed', tone: 'danger' })"
              >Fail Send</Button>
              <Button
                size="md"
                variant="ghost"
                @click="showToast({ title: 'Copied', tone: 'success' })"
              >Copy ID</Button>
              <Button
                size="md"
                variant="ghost"
                @click="showArchived(1, () => productWould('Restore the Conversation'))"
              >Archive</Button>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="Connection Banner"
        note="Across the top of the app while the backend cannot be reached, as Slack and Linear show one; everything under it stays readable and usable, and it goes once the page reaches the backend again. It says why in the user’s words: no network, Demi restarting (the backend said so as it closed the page’s channel), or reconnecting once a lost channel’s first new attempt failed too. A message sent meanwhile says it waits to be sent (Session › Turns › Offline)."
      >
        <div class="specimen-stack">
          <GallerySpecimen variant="offline" wide>
            <ConnectionBanner problem="offline" />
          </GallerySpecimen>
          <GallerySpecimen variant="restarting" wide>
            <ConnectionBanner problem="restarting" />
          </GallerySpecimen>
          <GallerySpecimen variant="reconnecting" wide>
            <ConnectionBanner problem="reconnecting" />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="Page Could Not Be Updated"
        note="Over the whole app once a page that loaded itself for the build the backend serves still got another one, as when a cache in front of Demi keeps an earlier page. It says so and offers Reload."
      >
        <div class="specimen-row specimen-row-wide items-start">
          <GalleryOverlayWell size="wide">
            <GallerySpecimen variant="could not be updated">
              <UpdateFailedScreen @reload="productWould('Reload the Page')" />
            </GallerySpecimen>
          </GalleryOverlayWell>
        </div>
      </GallerySection>

      <GallerySection
        title="Dialog"
        note="Modal confirm. The footer holds the actions the macOS way: Cancel, then the default button last and rightmost; what stands apart, a link or Delete beside Save, sits at the leading edge. Return presses the default button from anywhere in the panel but a button, a link or a multi-line field, and a dialog without a field opens with the focus on it; Escape cancels. An action that destroys (Remove, Revoke, Delete) is never the default, as Apple’s guidelines and macOS’s alerts have it: in a confirmation of one, Cancel is the primary button and has the focus, so Return cancels, and the action is red with no fill and takes a click. The footer stays in place while the content scrolls. The pinned one starts open. Show Toast keeps the live dialog open: the notification stays above its scrim and can be dismissed without closing the dialog."
      >
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen variant="open">
            <Button size="md" @click="dialogOpen = true">Open</Button>
          </GallerySpecimen>
        </div>
        <GalleryOverlayWell size="lg">
          <GallerySpecimen variant="pinned">
            <Button
              v-if="!inlineDialogOpen"
              size="md"
              @click="inlineDialogOpen = true"
            >Open</Button>
            <Dialog
              :is-open="inlineDialogOpen"
              :overlay-store="appOverlayStore"
              @close="inlineDialogOpen = false"
            >
              <div class="space-y-3 px-5 pt-5">
                <h3 class="text-[15px] font-medium text-fg-emphasis">Keep this queued follow-up?</h3>
                <p class="text-[13px] leading-5 text-fg-muted">
                  The expired-cookie case can wait. Keep the queued message for the next turn?
                </p>
              </div>
              <template #footer>
                <Button @click="inlineDialogOpen = false">Cancel</Button>
                <Button variant="primary" @click="inlineDialogOpen = false">Keep</Button>
              </template>
            </Dialog>
          </GallerySpecimen>
        </GalleryOverlayWell>
        <GalleryOverlayWell size="tall">
          <GallerySpecimen variant="layers · dialog, menu and submenu">
            <Button
              v-if="!layersDialogOpen"
              size="md"
              @click="layersDialogOpen = true"
            >Open</Button>
            <Dialog
              :is-open="layersDialogOpen"
              :overlay-store="appOverlayStore"
              @close="layersDialogOpen = false"
            >
              <div class="p-4">
                <SettingsGroup title="Edit Profile">
                  <SettingsRow label="Model settings">
                    <Segmented v-model="layersSource" size="sm" :options="layersSources" />
                  </SettingsRow>
                  <SettingsRow label="Model">
                    <Dropdown
                      v-model:open="layersMenuOpen"
                      variant="default"
                      size="sm"
                      :overlay-store="appOverlayStore"
                    >
                      <template #trigger>
                        {{ itemLabel(layersModel, submenuModels) }} · {{ itemLabel(layersReasoning, effortItems) }}
                      </template>
                      <template #content="{ close }">
                        <Menu iconless @click="close">
                          <MenuItem
                            submenu-open
                            label="Reasoning"
                            :value="itemLabel(layersReasoning, effortItems)"
                          >
                            <template #submenu>
                              <Menu iconless>
                                <MenuItem
                                  v-for="item in effortItems"
                                  :key="item.id"
                                  :label="item.label"
                                  choice
                                  :is-selected="layersReasoning === item.id"
                                  @select="layersReasoning = item.id"
                                />
                              </Menu>
                            </template>
                          </MenuItem>
                          <MenuItem
                            label="Model"
                            :value="itemLabel(layersModel, submenuModels)"
                          >
                            <template #submenu>
                              <Menu iconless>
                                <MenuItem
                                  v-for="item in submenuModels"
                                  :key="item.id"
                                  :label="item.label"
                                  choice
                                  :is-selected="layersModel === item.id"
                                  @select="layersModel = item.id"
                                />
                              </Menu>
                            </template>
                          </MenuItem>
                        </Menu>
                      </template>
                    </Dropdown>
                  </SettingsRow>
                </SettingsGroup>
              </div>
            </Dialog>
          </GallerySpecimen>
        </GalleryOverlayWell>
        <Dialog
          :is-open="dialogOpen"
          :overlay-store="appOverlayStore"
          @close="dialogOpen = false"
        >
          <div class="space-y-3 px-5 pt-5">
            <h3 class="text-[15px] font-medium text-fg-emphasis">Keep this queued follow-up?</h3>
            <p class="text-[13px] leading-5 text-fg-muted">
              The expired-cookie case can wait. Keep the queued message for the next turn?
            </p>
          </div>
          <template #footer-leading>
            <Button
              variant="ghost"
              @click="showToast({ title: 'Could Not Refresh Usage', message: 'The provider is temporarily unavailable.', tone: 'danger' })"
            >Show Toast</Button>
          </template>
          <template #footer>
            <Button @click="dialogOpen = false">Cancel</Button>
            <Button variant="primary" @click="dialogOpen = false">Keep</Button>
          </template>
        </Dialog>
      </GallerySection>
    </template>
  </div>
</template>
