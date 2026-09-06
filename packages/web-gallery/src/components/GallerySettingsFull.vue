<script setup lang="ts">
import { computed, ref } from 'vue'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { Check, Copy, ExternalLink, Eye, EyeOff, FolderOpen, Laptop, Monitor, Moon, Plug, RotateCw, ScrollText, Server, Sun, Terminal, Trash2 } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Meter from '@demicodes/web-ui/ui/Meter.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import ShortcutRecorder from '@demicodes/web-ui/ui/ShortcutRecorder.vue'
import SwatchPicker from '@demicodes/web-ui/ui/SwatchPicker.vue'
import { PRODUCT_ACCENTS, PRODUCT_TONES, type ProductAccent, type ProductTone } from '@demicodes/web-ui/theme/productAppearance'
import { galleryState } from '../gallery-state'
import Slider from '@demicodes/web-ui/ui/Slider.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextArea from '@demicodes/web-ui/ui/TextArea.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import SettingsNote from '@demicodes/web-ui/settings/SettingsNote.vue'
import SettingsPage from '@demicodes/web-ui/settings/SettingsPage.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import type { Permission, SettingsState } from '../fixtures/settings'
import GallerySettingsProviders from './GallerySettingsProviders.vue'

/** One page of the full mock, chosen by the dialog's tab. All state lives in the fixture. */
const props = defineProps<{
  tab: string
  state: SettingsState
}>()

const s = computed(() => props.state)

const permissionOptions = [
  { value: 'allow', label: 'Allow' },
  { value: 'ask', label: 'Ask' },
  { value: 'deny', label: 'Deny' },
] as const satisfies readonly { value: Permission; label: string }[]

const themeOptions = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'System', icon: Monitor },
] as const

const serverTone = { connected: 'success', auth: 'warning', crashed: 'danger', disabled: 'neutral' } as const
const serverWord = { connected: 'Connected', auth: 'Sign in', crashed: 'Crashed', disabled: 'Off' } as const

const tools = [
  { key: 'edit', label: 'Edit files', description: 'Create, change and delete files in the working directory.' },
  { key: 'shell', label: 'Run shell commands', description: 'Anything outside the allowlist below follows this rule.' },
  { key: 'read', label: 'Read files', description: 'Open files and list directories.' },
  { key: 'web', label: 'Fetch the web', description: 'Load pages and call HTTP APIs.' },
  { key: 'mcp', label: 'MCP tools', description: 'Tools from the servers you connected.' },
] as const

function pick<T extends string>(current: T, values: readonly T[], set: (value: T) => void) {
  return { current, values, set }
}

// Actions the mock cannot perform say what the product would do.
const note = (title: string, message?: string) => showToast({ title, message })
function copyText(text: string, what: string) {
  void navigator.clipboard?.writeText(text)
  note(`${what} copied`)
}

const checking = ref(false)
function checkUpdates() {
  checking.value = true
  window.setTimeout(() => {
    checking.value = false
    note(`Demi ${s.value.general.version} is up to date`)
  }, 900)
}

const newPattern = ref('')
function addPattern() {
  const value = newPattern.value.trim()
  if (!value) return
  if (!s.value.permissions.allowlist.includes(value)) s.value.permissions.allowlist.push(value)
  newPattern.value = ''
}

function resetPermissions() {
  Object.assign(s.value.permissions, { edit: 'ask', shell: 'ask', read: 'allow', web: 'allow', mcp: 'ask' })
  note('Permissions reset')
}

function createAgent() {
  const n = s.value.agents.length + 1
  s.value.agents.push({ id: `agent-${n}`, name: `Agent ${n}`, model: 'Claude Sonnet', mode: 'primary', summary: 'Full tool access · edits, shell, web', enabled: true, expanded: true })
}

const newServer = ref({ transport: 'stdio' as 'stdio' | 'http', target: '', name: '' })
function addServer() {
  const { transport, target, name } = newServer.value
  if (!target.trim() || !name.trim()) return
  s.value.servers.push({ id: `server-${Date.now()}`, name: name.trim(), transport, target: target.trim(), state: 'connected', enabled: true, expanded: false, tools: [] })
  newServer.value = { transport: 'stdio', target: '', name: '' }
}

function restartServer(server: SettingsState['servers'][number]) {
  server.state = 'connected'
  server.detail = undefined
  note(`${server.name} restarted`)
}

function signInServer(server: SettingsState['servers'][number]) {
  server.state = 'connected'
  server.detail = undefined
  note(`Signed in to ${server.name}`)
}

function revokeDevice(id: string) {
  s.value.devices = s.value.devices.filter((d) => d.id !== id)
}

/** A binding another action already holds is refused; the row keeps its old keys. */
function rebind(id: string, keys: string) {
  const list = s.value.keys
  const target = list.find((b) => b.id === id)
  if (!target) return
  const taken = list.find((b) => b.id !== id && b.keys === keys)
  if (taken) {
    showToast({ title: `${keys} is taken`, message: `Already bound to “${taken.action}”. Remove it there first.`, tone: 'danger' })
    return
  }
  target.keys = keys
}

const DEFAULT_KEYS: Record<string, string> = { new: '⌘N', send: '⏎', stop: '⎋', sidebar: '⌘B', search: '⌘K', focus: '⌘L', settings: '⌘,' }
function resetShortcuts() {
  for (const binding of s.value.keys) binding.keys = DEFAULT_KEYS[binding.id] ?? binding.keys
  note('Shortcuts reset')
}

function clearMemory() {
  s.value.instructions.memories = 0
  note('Memory cleared')
}

const revealed = ref<Record<string, boolean>>({})
</script>

<template>
  <!-- General -->
  <SettingsPage v-if="tab === 'general'" title="General" description="Language, look, and how the app starts.">
    <SettingsGroup title="Appearance">
      <SettingsRow label="Language" description="The app's own text. Model output is unaffected.">
        <Dropdown size="sm" :overlay-store="appOverlayStore" variant="default" trigger-label="Language">
          <template #trigger>{{ s.general.language }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="lang in ['English', '简体中文', '日本語']" :key="lang" :label="lang" choice :is-selected="s.general.language === lang" @select="s.general.language = lang; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Theme">
        <Segmented size="sm" v-model="s.general.theme" :options="themeOptions" />
      </SettingsRow>
      <!-- Tone and accent change the gallery itself, the way they would change the app. -->
      <SettingsRow label="Tone">
        <Segmented size="sm" :model-value="galleryState.tone" :options="PRODUCT_TONES.map((t) => ({ value: t.id, label: t.label }))" @update:model-value="galleryState.tone = $event as ProductTone" />
      </SettingsRow>
      <SettingsRow label="Accent">
        <SwatchPicker :model-value="galleryState.accent" :options="PRODUCT_ACCENTS" @update:model-value="galleryState.accent = $event as ProductAccent" />
      </SettingsRow>
      <SettingsRow label="Transcript text size" description="Messages only.">
        <Slider v-model="s.general.fontSize" :min="12" :max="18" :value-label="`${s.general.fontSize}px`" class="w-48" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Composer">
      <SettingsRow label="Send message with" description="The other combination inserts a line break.">
        <Segmented size="sm" v-model="s.general.sendWith" :options="[{ value: 'enter', label: 'Enter' }, { value: 'cmdEnter', label: '⌘ Enter' }]" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Startup & updates">
      <SettingsRow label="Open at login">
        <Switch v-model="s.general.openAtLogin" />
      </SettingsRow>
      <SettingsRow label="Update channel" description="Beta builds arrive about a week early.">
        <template #tags><Tag>{{ s.general.version }} · up to date</Tag></template>
        <Button size="sm" :disabled="checking" @click="checkUpdates">{{ checking ? 'Checking…' : 'Check now' }}</Button>
        <Dropdown size="sm" :overlay-store="appOverlayStore" variant="default" trigger-label="Update channel">
          <template #trigger>{{ s.general.channel }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="ch in ['Stable', 'Beta']" :key="ch" :label="ch" choice :is-selected="s.general.channel === ch" @select="s.general.channel = ch; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Account -->
  <SettingsPage v-else-if="tab === 'account'" title="Account" description="Who you are here, and what your plan covers.">
    <SettingsGroup title="Profile">
      <SettingsRow label="Avatar">
        <span class="flex size-8 items-center justify-center rounded-full bg-tint-accent text-[13px] font-medium text-on-accent">Z</span>
        <Button size="sm" @click="note('Choose a picture', 'The product opens a file picker here.')">Change</Button>
      </SettingsRow>
      <SettingsRow label="Display name">
        <TextInput v-model="s.account.name" class="w-56 max-w-full" />
      </SettingsRow>
      <SettingsRow label="Email">
        <template #tags><Tag tone="success">Verified</Tag></template>
        <span class="text-chrome text-fg-muted">{{ s.account.email }}</span>
        <Button size="sm" @click="note('Check your inbox', `A confirmation link went to ${s.account.email}.`)">Change</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Plan">
      <SettingsRow :label="`${s.account.plan} plan`" :description="`Renews ${s.account.renews}. Cancel any time before then.`">
        <template #tags><Tag tone="accent">Current</Tag></template>
        <Button size="sm" @click="note('Opening the billing portal')">Manage billing</Button>
      </SettingsRow>
      <div class="flex flex-col gap-2 px-4 py-3">
        <div class="flex items-baseline justify-between text-[12px]">
          <span class="select-none text-fg-muted">Monthly credits</span>
          <span class="tabular-nums text-fg">{{ s.account.creditsUsed.toLocaleString() }} / {{ s.account.creditsMax.toLocaleString() }}</span>
        </div>
        <Meter :value="s.account.creditsUsed" :max="s.account.creditsMax" label="Monthly credits" />
        <p class="select-none text-[12px] text-on-warning">86% used. Extra usage is billed at the end of the month.</p>
      </div>
    </SettingsGroup>
    <SettingsGroup title="Session">
      <SettingsRow label="Sign out" description="Conversations stay on the server.">
        <Button size="sm" @click="note('Signed out', 'The product returns to the login page.')">Sign out</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger zone">
      <SettingsRow label="Delete account" description="Removes your account, devices and every conversation. This cannot be undone.">
        <Button size="sm" variant="danger" @click="note('Delete your account?', 'The product asks you to type your email to confirm.')">Delete account</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Notifications -->
  <SettingsPage v-else-if="tab === 'notifications'" title="Notifications" description="When the agent needs you, or is done.">
    <SettingsGroup title="Delivery">
      <SettingsRow label="Desktop notifications">
        <Switch v-model="s.notifications.desktop" />
      </SettingsRow>
      <SettingsRow label="Sound" description="Only while the window is in the background.">
        <Switch v-model="s.notifications.sound" :class="s.notifications.desktop ? '' : 'pointer-events-none opacity-40'" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Notify me when" description="Only while the conversation is not in front.">
      <SettingsRow label="A turn finishes"><Checkbox v-model="s.notifications.onFinish" label="" /></SettingsRow>
      <SettingsRow label="A tool needs approval" description="Edits and shell commands set to Ask."><Checkbox v-model="s.notifications.onApproval" label="" /></SettingsRow>
      <SettingsRow label="A turn fails"><Checkbox v-model="s.notifications.onError" label="" /></SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Quiet hours">
      <SettingsRow label="Silence notifications" :description="s.notifications.quietHours ? `Between ${s.notifications.quietRange}. Approvals still come through.` : undefined">
        <Button size="sm" v-if="s.notifications.quietHours" @click="note('Pick the hours', 'The product opens a time-range picker.')">{{ s.notifications.quietRange }}</Button>
        <Switch v-model="s.notifications.quietHours" />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Models & providers -->
  <GallerySettingsProviders v-else-if="tab === 'models'" :state="state" />

  <!-- Agents -->
  <SettingsPage v-else-if="tab === 'agents'" title="Agents" description="Named setups the picker offers: a model, a mode and what it may touch.">
    <SettingsGroup title="Your agents">
      <template v-for="agent in s.agents" :key="agent.id">
        <SettingsRow :label="agent.name" :description="`${agent.model} · ${agent.summary}`" :class="agent.enabled ? '' : 'opacity-60'">
          <template #tags>
            <Tag v-if="agent.isDefault" tone="accent">Default</Tag>
            <Tag v-if="agent.mode === 'subagent'">Subagent</Tag>
          </template>
          <Button size="sm" @click="agent.expanded = !agent.expanded">{{ agent.expanded ? 'Done' : 'Edit' }}</Button>
          <Switch v-model="agent.enabled" size="sm" class="ml-1" />
        </SettingsRow>
        <template v-if="agent.expanded">
          <SettingsRow inset label="Model">
            <Dropdown :overlay-store="appOverlayStore" variant="default" size="sm" trigger-label="Agent model">
              <template #trigger>{{ agent.model }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem v-for="m in ['Claude Sonnet', 'Claude Haiku', 'GPT-5']" :key="m" :label="m" choice :is-selected="agent.model === m" @select="agent.model = m; close()" />
                </Menu>
              </template>
            </Dropdown>
          </SettingsRow>
          <SettingsRow inset label="Mode" description="A primary agent is picked per conversation; a subagent is delegated to.">
            <Segmented size="sm" v-model="agent.mode" :options="[{ value: 'primary', label: 'Primary' }, { value: 'subagent', label: 'Subagent' }]" />
          </SettingsRow>
          <SettingsRow inset label="Tools">
            <Tag tone="success">edit</Tag>
            <Tag tone="success">shell</Tag>
            <Tag tone="success">web</Tag>
            <Tag>mcp · ask</Tag>
          </SettingsRow>
          <SettingsRow inset label="System prompt" description="Appended after the global instructions.">
            <Button size="sm" @click="note('Prompt editor', 'The product opens the prompt in an editor dialog.')">Edit prompt</Button>
          </SettingsRow>
        </template>
      </template>
    </SettingsGroup>
    <SettingsGroup>
      <SettingsRow label="New agent" description="Starts from Build's settings.">
        <Button size="sm" @click="createAgent">Create</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Permissions -->
  <SettingsPage v-else-if="tab === 'permissions'" title="Permissions" description="What the agent may do without asking. Ask pauses the turn until you answer.">
    <SettingsGroup title="Tools">
      <SettingsRow v-for="tool in tools" :key="tool.key" :label="tool.label" :description="tool.description">
        <Segmented size="sm" v-model="s.permissions[tool.key]" :options="permissionOptions" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Shell allowlist" description="Commands matching a pattern run without asking, whatever the rule above says.">
      <SettingsRow v-for="pattern in s.permissions.allowlist" :key="pattern" inset :label="pattern">
        <template #leading><Terminal :size="ICON_PX.in24" /></template>
        <Tooltip content="Remove"><IconButton size="sm" :icon="Trash2" variant="danger" aria-label="Remove pattern" @click="s.permissions.allowlist = s.permissions.allowlist.filter((p) => p !== pattern)" /></Tooltip>
      </SettingsRow>
      <SettingsRow label="Add a pattern" description="Glob syntax. `git *` matches every git command.">
        <TextInput v-model="newPattern" placeholder="docker compose *" class="w-56 max-w-full" @keydown.enter="addPattern" />
        <Button size="sm" :disabled="!newPattern.trim()" @click="addPattern">Add</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Scope">
      <SettingsRow label="Apply rules to" description="Project rules live in .demi/permissions.json and travel with the checkout.">
        <Segmented size="sm" v-model="s.permissions.scope" :options="[{ value: 'project', label: 'This project' }, { value: 'everywhere', label: 'Everywhere' }]" />
      </SettingsRow>
      <SettingsRow label="Reset to defaults" description="Ask for edits and shell, allow reads and web.">
        <Button size="sm" @click="resetPermissions">Reset</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Instructions & memory -->
  <SettingsPage v-else-if="tab === 'instructions'" title="Instructions & memory" description="What every conversation starts knowing.">
    <SettingsGroup title="Global instructions" description="Prepended to every system prompt, in every project.">
      <div class="px-4 py-3">
        <TextArea v-model="s.instructions.global" :rows="4" placeholder="How should the agent work with you?" />
        <div class="mt-2 flex items-center justify-between text-[12px] text-fg-subtle">
          <span class="select-none">{{ s.instructions.global.length }} characters · about {{ Math.ceil(s.instructions.global.length / 4) }} tokens</span>
          <Button size="sm" @click="note('System prompt preview', `${s.instructions.global.length} characters of global instructions, then the project files.`)">Preview prompt</Button>
        </div>
      </div>
    </SettingsGroup>
    <SettingsGroup title="Project instructions" description="AGENTS.md files the agent reads from the working directory up to the checkout root.">
      <SettingsRow v-for="file in s.instructions.files" :key="file.path" :label="file.path" :description="file.found ? 'Read at the start of every turn.' : 'Not found. Create it to give this project its own rules.'">
        <template #tags><Tag :tone="file.found ? 'success' : 'neutral'">{{ file.found ? 'Found' : 'Missing' }}</Tag></template>
        <Tooltip v-if="file.found" content="Open in editor"><IconButton size="sm" :icon="ExternalLink" aria-label="Open in editor" @click="note('Opening in your editor', file.path)" /></Tooltip>
        <Button v-else size="sm" @click="file.found = true">Create</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Memory">
      <SettingsRow label="Remember across conversations" description="Facts you confirm are saved and offered back when relevant.">
        <Switch v-model="s.instructions.memory" />
      </SettingsRow>
      <SettingsRow label="Saved memories" :description="`${s.instructions.memories} entries · last added yesterday`">
        <Button size="sm" @click="note('Saved memories', 'The product lists them in a dialog with a remove button each.')">Manage</Button>
      </SettingsRow>
      <SettingsRow label="Clear memory" description="Forgets everything saved so far. Conversations are kept.">
        <Button size="sm" variant="danger" :disabled="!s.instructions.memories" @click="clearMemory">Clear</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- MCP servers -->
  <SettingsPage v-else-if="tab === 'mcp'" title="MCP servers" description="Tool servers the agent can call. Off keeps the config but hides the tools.">
    <SettingsGroup title="Servers">
      <template v-for="server in s.servers" :key="server.id">
        <SettingsRow :label="server.name" :class="server.enabled ? '' : 'opacity-60'">
          <template #leading><component :is="server.transport === 'stdio' ? Terminal : Server" :size="ICON_PX.in28" /></template>
          <template #tags>
            <Tag :tone="serverTone[server.state]">{{ serverWord[server.state] }}</Tag>
            <Tag v-if="server.tools.length">{{ server.tools.filter((t) => t.enabled).length }}/{{ server.tools.length }} tools</Tag>
          </template>
          <template #description>
            <span class="font-mono">{{ server.target }}</span>
            <span v-if="server.detail" class="block" :class="server.state === 'crashed' ? 'font-mono text-on-danger' : 'text-on-warning'">{{ server.detail }}</span>
          </template>
          <template v-if="server.state === 'auth'"><Button size="sm" @click="signInServer(server)">Sign in</Button></template>
          <template v-else-if="server.state === 'crashed'">
            <Tooltip content="Logs"><IconButton size="sm" :icon="ScrollText" aria-label="Show logs" @click="note(`${server.name} logs`, server.detail)" /></Tooltip>
            <Tooltip content="Restart"><IconButton size="sm" :icon="RotateCw" aria-label="Restart server" @click="restartServer(server)" /></Tooltip>
          </template>
          <template v-else-if="server.state === 'connected'">
            <Button size="sm" @click="server.expanded = !server.expanded">{{ server.expanded ? 'Hide tools' : 'Tools' }}</Button>
          </template>
          <Switch v-model="server.enabled" size="sm" class="ml-1" />
        </SettingsRow>
        <template v-if="server.expanded">
          <SettingsRow v-for="tool in server.tools" :key="tool.name" inset :label="tool.name">
            <template #leading><Plug :size="ICON_PX.in24" /></template>
            <Switch v-model="tool.enabled" size="sm" />
          </SettingsRow>
        </template>
      </template>
    </SettingsGroup>
    <SettingsGroup title="Add a server">
      <SettingsRow label="Transport">
        <Segmented size="sm" v-model="newServer.transport" :options="[{ value: 'stdio', label: 'Command' }, { value: 'http', label: 'URL' }]" />
      </SettingsRow>
      <SettingsRow :label="newServer.transport === 'stdio' ? 'Command' : 'URL'" :description="newServer.transport === 'stdio' ? 'Run from the project root. Environment variables from Developer are passed through.' : undefined">
        <TextInput v-model="newServer.target" :placeholder="newServer.transport === 'stdio' ? 'npx -y @modelcontextprotocol/server-memory' : 'https://mcp.example.com'" class="w-72 max-w-full" />
      </SettingsRow>
      <SettingsRow label="Name" description="How tools show up: name_tool.">
        <TextInput v-model="newServer.name" placeholder="memory" class="w-40 max-w-full" @keydown.enter="addServer" />
        <Button size="sm" :disabled="!newServer.target.trim() || !newServer.name.trim()" @click="addServer">Add server</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Devices -->
  <SettingsPage v-else-if="tab === 'devices'" title="Devices" description="Machines that can host a conversation's working directory.">
    <SettingsGroup title="Your devices">
      <template v-for="device in s.devices" :key="device.id">
        <SettingsRow :label="device.name" :description="device.online ? `Online · Demi ${device.version}` : `Last seen ${device.seen} · Demi ${device.version}`">
          <template #leading>
            <span class="relative flex">
              <component :is="device.current ? Laptop : Monitor" :size="ICON_PX.in28" />
              <span class="absolute -right-0.5 -top-0.5 size-1.5 rounded-full ring-2 ring-surface-float" :class="device.online ? 'bg-on-success' : 'bg-fg-ghost'" />
            </span>
          </template>
          <template #tags>
            <Tag v-if="device.current" tone="accent">This device</Tag>
            <Tag v-if="device.version !== '1.6.2'" tone="warning">Update available</Tag>
          </template>
          <!-- Presence is the device's own doing; the only action here is to revoke it. -->
          <Tooltip content="Revoke"><IconButton size="sm" :icon="Trash2" variant="danger" :disabled="device.current" aria-label="Revoke device" @click="revokeDevice(device.id)" /></Tooltip>
        </SettingsRow>
      </template>
    </SettingsGroup>
    <SettingsGroup title="Add a device" description="Install Demi on the machine and paste this code; it appears above once it connects.">
      <SettingsRow label="Pairing code" description="Expires in 9 minutes.">
        <span class="font-mono text-[15px] tracking-[0.2em] text-fg">7KQ-42M</span>
        <IconButton size="sm" :icon="Copy" variant="ghost" aria-label="Copy pairing code" @click="copyText('7KQ-42M', 'Pairing code')" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsNote text="Remove the projects using a device before revoking it." />
  </SettingsPage>

  <!-- Keyboard -->
  <SettingsPage v-else-if="tab === 'keyboard'" title="Keyboard" description="Click one to change it.">
    <SettingsGroup title="Shortcuts">
      <SettingsRow v-for="binding in s.keys" :key="binding.id" :label="binding.action">
        <ShortcutRecorder :model-value="binding.keys" size="sm" @update:model-value="rebind(binding.id, $event)" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup>
      <SettingsRow label="Reset all shortcuts">
        <Button size="sm" @click="resetShortcuts">Reset</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Data & privacy -->
  <SettingsPage v-else-if="tab === 'data'" title="Data & privacy" description="What is kept, for how long, and who can see it.">
    <SettingsGroup title="Conversations">
      <SettingsRow label="Keep transcripts for" description="Older conversations are deleted from every device.">
        <Dropdown size="sm" :overlay-store="appOverlayStore" variant="default" trigger-label="Retention">
          <template #trigger>{{ s.data.retention }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="r in ['Forever', '90 days', '30 days', '7 days']" :key="r" :label="r" choice :is-selected="s.data.retention === r" @select="s.data.retention = r; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Share links" description="Let a conversation be published at a public URL.">
        <Switch v-model="s.data.shareLinks" />
      </SettingsRow>
      <SettingsRow label="Export everything" description="Transcripts, settings and memories as a zip. Ready in a few minutes.">
        <Button size="sm" @click="note('Export requested', 'A download link arrives by email in a few minutes.')">Request export</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Diagnostics">
      <SettingsRow label="Send usage data" description="Crashes and feature usage. Never prompts, transcripts or file contents.">
        <Switch v-model="s.data.telemetry" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger zone">
      <SettingsRow label="Delete all conversations" description="On every device. Projects and settings stay.">
        <Button size="sm" variant="danger" @click="note('Delete every conversation?', 'The product asks you to confirm once more.')">Delete all</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Usage & billing -->
  <SettingsPage v-else-if="tab === 'usage'" title="Usage & billing" description="This month, across every provider.">
    <SettingsGroup title="Plan">
      <SettingsRow :label="`${s.account.plan} plan`" :description="`Next invoice ${s.account.renews} · credits reset ${s.usage.resets}`">
        <Button size="sm" @click="note('Opening your invoices')">Invoices</Button>
        <Button size="sm" @click="note('Opening the billing portal')">Manage billing</Button>
      </SettingsRow>
      <div class="flex flex-col gap-2 px-4 py-3">
        <div class="flex items-baseline justify-between text-[12px]">
          <span class="select-none text-fg-muted">Monthly credits</span>
          <span class="tabular-nums text-fg">{{ s.account.creditsUsed.toLocaleString() }} / {{ s.account.creditsMax.toLocaleString() }}</span>
        </div>
        <Meter :value="s.account.creditsUsed" :max="s.account.creditsMax" label="Monthly credits" />
      </div>
    </SettingsGroup>
    <SettingsGroup title="Spend by provider" description="Billed by the providers you connected with your own keys.">
      <SettingsRow v-for="row in s.usage.spend" :key="row.provider" :label="row.provider">
        <span class="text-chrome tabular-nums text-fg">{{ row.amount }}</span>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Tokens">
      <SettingsRow label="Input"><span class="text-chrome tabular-nums text-fg">{{ s.usage.input }}</span></SettingsRow>
      <SettingsRow label="Output"><span class="text-chrome tabular-nums text-fg">{{ s.usage.output }}</span></SettingsRow>
      <SettingsRow label="Cache reads" description="Served from prompt cache; billed at a tenth of input."><span class="text-chrome tabular-nums text-fg">{{ s.usage.cacheRead }}</span></SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Developer -->
  <SettingsPage v-else-if="tab === 'developer'" title="Developer" description="Logs, the config file and features that are not finished.">
    <SettingsGroup title="Logging">
      <SettingsRow label="Log level" description="Debug writes every provider request. Large.">
        <Dropdown size="sm" :overlay-store="appOverlayStore" variant="default" trigger-label="Log level">
          <template #trigger>{{ s.developer.logLevel }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="l in ['Error', 'Warn', 'Info', 'Debug']" :key="l" :label="l" choice :is-selected="s.developer.logLevel === l" @select="s.developer.logLevel = l; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Open log folder">
        <Tooltip content="Open folder"><IconButton size="sm" :icon="FolderOpen" aria-label="Open log folder" @click="note('Opening the log folder', '~/Library/Logs/Demi')" /></Tooltip>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Configuration">
      <SettingsRow label="Config file" description="Edits made here are written back; edits made there reload live.">
        <span class="font-mono text-[12px] text-fg-muted">{{ s.developer.configPath }}</span>
        <IconButton size="sm" :icon="Copy" variant="ghost" aria-label="Copy config path" @click="copyText(s.developer.configPath, 'Path')" />
        <Tooltip content="Open in editor"><IconButton size="sm" :icon="ExternalLink" aria-label="Open config in editor" @click="note('Opening in your editor', s.developer.configPath)" /></Tooltip>
      </SettingsRow>
      <SettingsRow v-for="entry in s.developer.env" :key="entry.key" inset :label="entry.key">
        <span class="font-mono text-[12px] text-fg-muted">{{ revealed[entry.key] ? entry.value.replace(/•+/, '3f2a9c1d7e5b4a6f8c2d1e9b') : entry.value }}</span>
        <Tooltip :content="revealed[entry.key] ? 'Hide' : 'Reveal'"><IconButton size="sm" :icon="revealed[entry.key] ? EyeOff : Eye" :aria-label="revealed[entry.key] ? 'Hide value' : 'Reveal value'" @click="revealed[entry.key] = !revealed[entry.key]" /></Tooltip>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Experiments" description="May change or disappear. Feedback welcome.">
      <SettingsRow v-for="exp in s.developer.experiments" :key="exp.id" :label="exp.name" :description="exp.description">
        <template #tags><Tag tone="accent">Beta</Tag></template>
        <Switch v-model="exp.on" />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
