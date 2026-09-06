<script setup lang="ts">
import { computed } from 'vue'
import { Check, Copy, ExternalLink, Laptop, Monitor, Moon, Plug, Server, Sun, Terminal } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import KeyCap from '@demicodes/web-ui/ui/KeyCap.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Meter from '@demicodes/web-ui/ui/Meter.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import Slider from '@demicodes/web-ui/ui/Slider.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextArea from '@demicodes/web-ui/ui/TextArea.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
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
</script>

<template>
  <!-- General -->
  <SettingsPage v-if="tab === 'general'" title="General" description="Language, look, and how the app starts.">
    <SettingsGroup title="Appearance">
      <SettingsRow label="Language" description="Menus and messages from the app. Model output is unaffected.">
        <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Language">
          <template #trigger>{{ s.general.language }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="lang in ['English', '简体中文', '日本語']" :key="lang" :label="lang" choice :is-selected="s.general.language === lang" @select="s.general.language = lang; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Theme" description="System follows the OS and switches with it.">
        <Segmented v-model="s.general.theme" :options="themeOptions" />
      </SettingsRow>
      <SettingsRow label="Density" description="Spacing of lists, rows and the transcript.">
        <Segmented
          v-model="s.general.density"
          :options="[{ value: 'compact', label: 'Compact' }, { value: 'regular', label: 'Regular' }, { value: 'comfortable', label: 'Comfortable' }]"
        />
      </SettingsRow>
      <SettingsRow label="Transcript text size" description="Messages only. The rest of the app keeps its size.">
        <Slider v-model="s.general.fontSize" :min="12" :max="18" :value-label="`${s.general.fontSize}px`" class="w-48" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Composer">
      <SettingsRow label="Send message with" description="The other combination inserts a line break.">
        <Segmented v-model="s.general.sendWith" :options="[{ value: 'enter', label: 'Enter' }, { value: 'cmdEnter', label: '⌘ Enter' }]" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Startup & updates">
      <SettingsRow label="Open at login" description="Start Demi when you sign in to this Mac.">
        <Switch v-model="s.general.openAtLogin" />
      </SettingsRow>
      <SettingsRow label="Update channel" description="Beta builds arrive about a week early.">
        <template #tags><Tag>{{ s.general.version }} · up to date</Tag></template>
        <Button size="sm">Check now</Button>
        <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Update channel">
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
      <SettingsRow label="Avatar" description="Shown in the sidebar and on shared transcripts.">
        <span class="flex size-8 items-center justify-center rounded-full bg-tint-accent text-[13px] font-medium text-on-accent">Z</span>
        <Button size="sm">Change</Button>
      </SettingsRow>
      <SettingsRow label="Display name">
        <TextInput v-model="s.account.name" class="w-56 max-w-full" />
      </SettingsRow>
      <SettingsRow label="Email" description="Sign-in and receipts go here.">
        <template #tags><Tag tone="success">Verified</Tag></template>
        <span class="text-chrome text-fg-muted">{{ s.account.email }}</span>
        <Button size="sm">Change</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Plan">
      <SettingsRow :label="`${s.account.plan} plan`" :description="`Renews ${s.account.renews}. Cancel any time before then.`">
        <template #tags><Tag tone="accent">Current</Tag></template>
        <Button>Manage billing</Button>
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
      <SettingsRow label="Sign out" description="Ends this browser's session. Conversations stay on the server.">
        <Button>Sign out</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger zone">
      <SettingsRow label="Delete account" description="Removes your account, devices and every conversation. This cannot be undone.">
        <Button variant="danger">Delete account</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Notifications -->
  <SettingsPage v-else-if="tab === 'notifications'" title="Notifications" description="When the agent needs you, or is done.">
    <SettingsGroup title="Delivery">
      <SettingsRow label="Desktop notifications" description="Through the system notification center.">
        <Switch v-model="s.notifications.desktop" />
      </SettingsRow>
      <SettingsRow label="Sound" description="Plays once per event while the window is in the background.">
        <Switch v-model="s.notifications.sound" :class="s.notifications.desktop ? '' : 'pointer-events-none opacity-40'" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Notify me when" description="Only while the conversation is not in front.">
      <SettingsRow label="A turn finishes"><Checkbox v-model="s.notifications.onFinish" label="" /></SettingsRow>
      <SettingsRow label="A tool needs approval" description="Edits and shell commands set to Ask."><Checkbox v-model="s.notifications.onApproval" label="" /></SettingsRow>
      <SettingsRow label="A turn fails"><Checkbox v-model="s.notifications.onError" label="" /></SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Quiet hours">
      <SettingsRow label="Silence notifications" :description="s.notifications.quietHours ? `Between ${s.notifications.quietRange}. Approvals still come through.` : 'Off.'">
        <Button v-if="s.notifications.quietHours" size="sm">{{ s.notifications.quietRange }}</Button>
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
            <Segmented v-model="agent.mode" size="sm" :options="[{ value: 'primary', label: 'Primary' }, { value: 'subagent', label: 'Subagent' }]" />
          </SettingsRow>
          <SettingsRow inset label="Tools">
            <Tag tone="success">edit</Tag>
            <Tag tone="success">shell</Tag>
            <Tag tone="success">web</Tag>
            <Tag>mcp · ask</Tag>
          </SettingsRow>
          <SettingsRow inset label="System prompt" description="Appended after the global instructions.">
            <Button size="sm">Edit prompt</Button>
          </SettingsRow>
        </template>
      </template>
    </SettingsGroup>
    <SettingsGroup>
      <SettingsRow label="Create an agent" description="Starts from Build's settings.">
        <Button>New agent</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Permissions -->
  <SettingsPage v-else-if="tab === 'permissions'" title="Permissions" description="What the agent may do without asking. Ask pauses the turn until you answer.">
    <SettingsGroup title="Tools">
      <SettingsRow v-for="tool in tools" :key="tool.key" :label="tool.label" :description="tool.description">
        <Segmented v-model="s.permissions[tool.key]" size="sm" :options="permissionOptions" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Shell allowlist" description="Commands matching a pattern run without asking, whatever the rule above says.">
      <SettingsRow v-for="pattern in s.permissions.allowlist" :key="pattern" inset :label="pattern">
        <template #leading><Terminal :size="ICON_PX.in24" /></template>
        <Button size="sm" @click="s.permissions.allowlist = s.permissions.allowlist.filter((p) => p !== pattern)">Remove</Button>
      </SettingsRow>
      <SettingsRow label="Add a pattern" description="Glob syntax. `git *` matches every git command.">
        <TextInput placeholder="docker compose *" class="w-56 max-w-full" />
        <Button disabled>Add</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Scope">
      <SettingsRow label="These rules apply to" description="Project rules live in .demi/permissions.json and travel with the checkout.">
        <Segmented v-model="s.permissions.scope" :options="[{ value: 'project', label: 'This project' }, { value: 'everywhere', label: 'Everywhere' }]" />
      </SettingsRow>
      <SettingsRow label="Reset to defaults" description="Ask for edits and shell, allow reads and web.">
        <Button size="sm">Reset</Button>
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
          <Button size="sm">Preview prompt</Button>
        </div>
      </div>
    </SettingsGroup>
    <SettingsGroup title="Project instructions" description="AGENTS.md files the agent reads from the working directory up to the checkout root.">
      <SettingsRow v-for="file in s.instructions.files" :key="file.path" :label="file.path" :description="file.found ? 'Read at the start of every turn.' : 'Not found. Create it to give this project its own rules.'">
        <template #tags><Tag :tone="file.found ? 'success' : 'neutral'">{{ file.found ? 'Found' : 'Missing' }}</Tag></template>
        <Button size="sm">{{ file.found ? 'Open' : 'Create' }}</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Memory">
      <SettingsRow label="Remember across conversations" description="Facts you confirm are saved and offered back when relevant.">
        <Switch v-model="s.instructions.memory" />
      </SettingsRow>
      <SettingsRow label="Saved memories" :description="`${s.instructions.memories} entries · last added yesterday`">
        <Button size="sm">Manage</Button>
      </SettingsRow>
      <SettingsRow label="Clear memory" description="Forgets everything saved so far. Conversations are kept.">
        <Button variant="danger">Clear</Button>
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
          <template v-if="server.state === 'auth'"><Button size="sm">Sign in</Button></template>
          <template v-else-if="server.state === 'crashed'">
            <Button size="sm">Logs</Button>
            <Button size="sm">Restart</Button>
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
        <Segmented model-value="stdio" :options="[{ value: 'stdio', label: 'Command' }, { value: 'http', label: 'URL' }]" />
      </SettingsRow>
      <SettingsRow label="Command" description="Run from the project root. Environment variables from Developer are passed through.">
        <TextInput placeholder="npx -y @modelcontextprotocol/server-memory" class="w-72 max-w-full" />
      </SettingsRow>
      <SettingsRow label="Name" description="How tools show up: name_tool.">
        <TextInput placeholder="memory" class="w-40 max-w-full" />
        <Button disabled>Add server</Button>
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
          <Button v-if="!device.current" size="sm">{{ device.online ? 'Go offline' : 'Connect' }}</Button>
          <Button size="sm" :class="device.current ? 'pointer-events-none opacity-40' : ''">Revoke</Button>
        </SettingsRow>
        <SettingsRow inset label="Home directory">
          <span class="font-mono text-[12px] text-fg-muted">{{ device.home }}</span>
        </SettingsRow>
      </template>
    </SettingsGroup>
    <SettingsGroup title="Add a device" description="Install Demi on the machine and paste this code; it appears above once it connects.">
      <SettingsRow label="Pairing code" description="Expires in 9 minutes.">
        <span class="font-mono text-[15px] tracking-[0.2em] text-fg">7KQ-42M</span>
        <IconButton :icon="Copy" variant="ghost" aria-label="Copy pairing code" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsNote text="Remove the projects using a device before revoking it." />
  </SettingsPage>

  <!-- Keyboard -->
  <SettingsPage v-else-if="tab === 'keyboard'" title="Keyboard" description="Shortcuts across the app. Click one to change it.">
    <SettingsGroup title="Shortcuts">
      <SettingsRow v-for="binding in s.keys" :key="binding.id" :label="binding.action" :description="binding.conflict ? `Also bound to ${binding.conflict}. The first match wins.` : undefined">
        <template #tags><Tag v-if="binding.conflict" tone="danger">Conflict</Tag></template>
        <KeyCap :keys="binding.keys" />
        <Button size="sm">Change</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup>
      <SettingsRow label="Reset all shortcuts" description="Back to the defaults above.">
        <Button size="sm">Reset</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Data & privacy -->
  <SettingsPage v-else-if="tab === 'data'" title="Data & privacy" description="What is kept, for how long, and who can see it.">
    <SettingsGroup title="Conversations">
      <SettingsRow label="Keep transcripts" description="Older conversations are deleted from every device.">
        <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Retention">
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
        <Button>Request export</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Diagnostics">
      <SettingsRow label="Send usage data" description="Crashes and feature usage. Never prompts, transcripts or file contents.">
        <Switch v-model="s.data.telemetry" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger zone">
      <SettingsRow label="Delete all conversations" description="On every device. Projects and settings stay.">
        <Button variant="danger">Delete all</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>

  <!-- Usage & billing -->
  <SettingsPage v-else-if="tab === 'usage'" title="Usage & billing" description="This month, across every provider.">
    <SettingsGroup title="Plan">
      <SettingsRow :label="`${s.account.plan} plan`" :description="`Next invoice ${s.account.renews} · credits reset ${s.usage.resets}`">
        <Button size="sm">Invoices</Button>
        <Button>Manage billing</Button>
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
        <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Log level">
          <template #trigger>{{ s.developer.logLevel }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="l in ['Error', 'Warn', 'Info', 'Debug']" :key="l" :label="l" choice :is-selected="s.developer.logLevel === l" @select="s.developer.logLevel = l; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Open log folder">
        <Button size="sm">
          Open
          <ExternalLink :size="ICON_PX.in24" />
        </Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Configuration">
      <SettingsRow label="Config file" description="Edits made here are written back; edits made there reload live.">
        <span class="font-mono text-[12px] text-fg-muted">{{ s.developer.configPath }}</span>
        <IconButton :icon="Copy" variant="ghost" aria-label="Copy config path" />
        <Button size="sm">Open</Button>
      </SettingsRow>
      <SettingsRow v-for="entry in s.developer.env" :key="entry.key" inset :label="entry.key">
        <span class="font-mono text-[12px] text-fg-muted">{{ entry.value }}</span>
        <Button size="sm">Reveal</Button>
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
