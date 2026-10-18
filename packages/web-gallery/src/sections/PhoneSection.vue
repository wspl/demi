<script setup lang="ts">
import { ref } from 'vue'
import GallerySection from '../components/GallerySection.vue'
import GalleryPhoneFrame from '../components/GalleryPhoneFrame.vue'

/** The scenes the page shows, each a live phone of its own. */
const CONVERSATION = [
  { scene: 'conversation', label: 'A turn running' },
  { scene: 'title', label: 'The title sheet' },
  { scene: 'drawer', label: 'The sidebar' },
  { scene: 'model', label: 'The model picker' },
  { scene: 'long-model', label: 'A long model name' },
  { scene: 'long-model', label: 'A long model name at 320 px', device: 'iphone-se' as const },
  { scene: 'passkey', label: 'The passkey offer after a handoff' },
  { scene: 'password', label: 'Choose Your Password at the first sign-in' },
]
/** How many phones have loaded: the next one loads then, so that the app's files load once, not all at once. */
const loaded = ref(0)
/**
 * The phone's metrics (`base.css` and `PHONE_ICON_PX` in `icon-metrics.ts`,
 * which every component reads): each control as a phone draws it.
 */
const METRICS = [
  { control: 'Toolbar and action buttons: the bar’s, Attach, Send, Stop, Scroll to Bottom, the context', face: '40 px', icon: '20 px', text: '—' },
  { control: 'An answer’s actions: Copy, Fork, Share, Regenerate, the version switcher', face: '36 px, 8 px apart', icon: '18 px', text: 'time 13 px' },
  { control: 'A small icon button, such as a tab’s Close', face: '28 px', icon: '16 px', text: '—' },
  { control: 'Buttons with a label', face: '36 px (small), 40 px, 44 px (large)', icon: '16 px', text: '15 px' },
  { control: 'Work rows, Settings’ rows and list items', face: '44 px', icon: '18 px', text: '15 px; Settings 17 px' },
  { control: 'The work panel’s, Agents’ and Running’s tabs, in a 44 px bar', face: '36 px, 4 px above and below', icon: '18 px', text: '15 px' },
  { control: 'Chips under the composer: the model, the workflow, Agents, Running', face: '32 px', icon: '16 px', text: '15 px' },
  { control: 'File-change pills under a work row', face: '28 px, 8 px apart', icon: '16 px', text: '13 px' },
  { control: 'Segmented controls', face: '36 px', icon: '18 px', text: '15 px' },
  { control: 'Form fields', face: '44 px', icon: '—', text: '16 px' },
  { control: 'Switches', face: '51 × 31 px, as iOS’s', icon: '—', text: '—' },
  { control: 'Settings’ descriptions', face: '—', icon: '—', text: '15 px' },
  { control: 'The sidebar and its group labels, Settings’ sections, the file tree, a folder’s list, Search’s rows', face: '44 px rows, 32 px actions', icon: '18 px', text: '16–17 px' },
  { control: 'Every menu’s rows, a sheet’s or one beside its field, such as a path’s completions', face: '44 px', icon: '18 px', text: '16 px' },
  { control: 'A path’s crumbs, chevrons between them', face: '36 px tall, 36 px wide at least', icon: '16 px', text: '15 px' },
  { control: 'Bars over a view: the toolbar (two rows where one would crowd), a file’s bar and header, a window’s status bar', face: '44 px', icon: '16–20 px', text: '15 px; facts 13 px' },
  { control: 'A link in running text', face: 'its line’s height, the text laid out as on the desktop', icon: '—', text: 'the prose’s' },
  { control: 'What a dialog, a card or a sign-in page explains; a fact or a hint beside controls; a tag', face: '—', icon: '—', text: '15 px; 13 px; 13 px' },
  { control: 'A device’s number badge', face: '—', icon: '—', text: '11 px' },
  { control: 'The transcript’s prose; the composer’s text', face: '—', icon: '—', text: 'the user’s Message text size; 16 px at least' },
]
const PANES = [
  { scene: 'change', label: 'Work panel · Change' },
  { scene: 'file', label: 'Work panel · File' },
  { scene: 'tree', label: 'Work panel · File tree' },
  { scene: 'browser', label: 'Work panel · Browser' },
  { scene: 'agents', label: 'Agents' },
  { scene: 'running', label: 'Running' },
  { scene: 'agents-failed', label: 'Agents · a failed child selected' },
  { scene: 'settings', label: 'Settings' },
  { scene: 'notifications', label: 'Settings › Notifications in Safari on iPhone' },
]
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection
      title="The Conversation"
      note="Below 768 px the app is a phone’s, and that width is the one switch for everything below: a touch screen in the desktop layout keeps the desktop’s. One pane shows at a time, and nothing of the page zooms, drifts or bounces. One 44 px bar under the status bar holds the sidebar’s button, three lines named Conversations, the conversation’s title with a chevron, and the work panel’s button, a rounded square holding how many tabs the panel has, as a phone browser’s tab button, named Tabs, 3 open, and empty and named Tabs with none; the title opens the title sheet, which offers the sidebar row’s menu for the conversation (Rename, Copy Conversation ID, Pin, Archive, Delete…, the one menu both use) and where it runs, and a dot before the title says the Host is not ready. The sidebar is a drawer from the left, 85% of the screen and at most 320 px, over a scrim; a swipe right from the screen’s left edge draws it in after the finger, and a tap on the scrim, a swipe left, Escape and Back close it. Every row is a finger’s height: the entries, the group labels’ buttons, the projects and the conversations 44 px with 16 px titles, their dots and marks scaled with them, the account and Settings at the foot 52 px; the controls a hover reveals on a desktop show at rest. Menus open as bottom sheets with the same items and no heading, as an iOS menu has none: a tap on the scrim, a drag down on the grabber past 30% of the sheet or a flick, Escape and Back close one, and it springs back from a shorter pull. The composer sits above the home indicator, and above the keyboard while it is open. Each phone is live: every control acts as in the product, but Back, since a page inside these frames leaves the browser’s history to the gallery around it."
    >
      <div class="flex flex-wrap gap-8">
        <GalleryPhoneFrame
          v-for="(item, index) in CONVERSATION"
          :key="item.label"
          :scene="item.scene"
          :device="item.device"
          :label="item.label"
          :active="index <= loaded"
          @loaded="loaded += 1"
        />
      </div>
    </GallerySection>
    <GallerySection
      title="One Phone Scale"
      note="A phone draws every control at a finger’s size, not the desktop’s with a larger invisible target, as iOS’s own apps draw theirs: these are the sizes, one set every component reads. A control drawn at 36 px or more is its own target, and its neighbours stand far enough off that no two share one; only a smaller control, such as a tab’s Close, still reaches past its edges to 44 px. The desktop keeps its own sizes."
    >
      <table class="w-full max-w-4xl text-left text-[13px]">
        <thead class="text-fg-subtle">
          <tr>
            <th class="py-1.5 pr-4 font-medium">Control</th>
            <th class="py-1.5 pr-4 font-medium">Size</th>
            <th class="py-1.5 pr-4 font-medium">Icon</th>
            <th class="py-1.5 font-medium">Text</th>
          </tr>
        </thead>
        <tbody class="text-fg-body">
          <tr v-for="metric in METRICS" :key="metric.control" class="border-t border-line-subtle">
            <td class="py-1.5 pr-4">{{ metric.control }}</td>
            <td class="py-1.5 pr-4 whitespace-nowrap">{{ metric.face }}</td>
            <td class="py-1.5 pr-4 whitespace-nowrap">{{ metric.icon }}</td>
            <td class="py-1.5">{{ metric.text }}</td>
          </tr>
        </tbody>
      </table>
    </GallerySection>
    <GallerySection
      title="Panels and Windows"
      note="The work panel opens full screen over the conversation, sliding in from the right, under its own bar: a back button, its chevron alone, named Back to Conversation, which goes back to the conversation where it was, as does a swipe right from the screen’s left edge, the panel following the finger over the conversation and going on past 30% of the width or with a flick, else springing back, the tabs as a strip that scrolls sideways, each tab, Changes and Files too, 36 px tall with room above and below it in the 44 px bar, as on the desktop, as wide as its title up to 160 px, and the selected one scrolled into view, and the strip’s controls; in the File view the tree covers the file’s whole width, its rows 44 px, and slides away when a file is picked. A file a message names opens there the same way. The Agents and Running windows rise as full-screen sheets with their tabs and Close; their tabs narrow to their icons, each with its status dot, 44 px wide at least, and only the selected tab adds its name, cut short, so five agents fit across the screen as they would not by name (a child that failed keeps its tab while it is selected, as the phone after Running shows; a command that was stopped shows alone); a long press opens a tab’s menu and the strip scrolls sideways with the selected tab in view. Back closes each, as it closes a sheet. Settings fills the screen as iOS Settings does, its sections 44 px rows."
    >
      <div class="flex flex-wrap gap-8">
        <GalleryPhoneFrame
          v-for="(item, index) in PANES"
          :key="item.scene"
          :scene="item.scene"
          :label="item.label"
          :active="CONVERSATION.length + index <= loaded"
          @loaded="loaded += 1"
        />
      </div>
    </GallerySection>
  </div>
</template>
