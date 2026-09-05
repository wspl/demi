<script setup lang="ts">
import GallerySection from '../components/GallerySection.vue'

const shipped = [
  ['Button / IconButton', '28px hit. default, primary, ghost; send is the accent disc.'],
  ['Switch / Checkbox', 'On product surfaces. ToggleSwitch and Slider stay in the library.'],
  ['TextInput', 'Single-line 28px field.'],
  ['Dropdown + Menu', 'Dropdown owns open/close; Menu is the panel.'],
  ['Dialog / ContextMenu / Tooltip / Popover', 'One overlay family.'],
  ['Toast / ErrorBlock', 'Leave-the-control notices and session errors. ErrorBox and InlineError stay in the library.'],
  ['IndeterminateSpinner', 'In-progress chrome. Scroll-to-bottom is an IconButton circle.'],
  ['Session tabs', 'Agent session tabs only.'],
  ['Session', 'Transcript and composer on one surface. File drop on the shell. Send does not call a real model.'],
  ['MarkdownPreview / CodePreview', 'Own routes. Full-pane previews.'],
  ['Transcript blocks', 'User, assistant, thinking, shell tools, error, abort, and compaction.'],
]

const next = [
  ['Sidebar', 'Entries at the top, plain conversations, then every project folded under its name and host, the account at the bottom. Designed on the Sidebar page; not in the product yet.'],
  ['Radio / RadioGroup', 'Settings and filters.'],
  ['Badge / Tag / Chip', 'Model capability and session state.'],
  ['Avatar', 'User and assistant marks. May stay omitted.'],
  ['Separator', 'Settings groups and list breaks.'],
  ['Skeleton', 'First-load placeholders outside the session wait row.'],
  ['Progress', 'Linear bar alongside the context ring.'],
  ['Kbd', 'Shortcut glyphs in menus.'],
  ['Tabs / Segmented control', 'N-item tabs outside the session strip.'],
  ['Card', 'Settings groups.'],
  ['Drawer / Sheet', 'Narrow widths or an inspector.'],
  ['Command palette', '⌘K.'],
  ['Empty state', 'List empty copy beyond a single session sentence.'],
]
</script>

<template>
  <div class="space-y-8">
    <GallerySection title="Shipped" note="What the catalog covers today. Session chrome is in; settings and system feedback are next.">
      <div class="gallery-frame divide-y divide-line">
        <div v-for="[name, note] in shipped" :key="name" class="grid gap-2 px-4 py-3 md:grid-cols-[220px_1fr]">
          <div class="text-[13px] text-fg">{{ name }}</div>
          <div class="text-[13px] text-fg-muted">{{ note }}</div>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Next">
      <div class="gallery-frame divide-y divide-line">
        <div v-for="[name, note] in next" :key="name" class="grid gap-2 px-4 py-3 md:grid-cols-[220px_1fr]">
          <div class="text-[13px] text-fg-emphasis">{{ name }}</div>
          <div class="text-[13px] text-fg-muted">{{ note }}</div>
        </div>
      </div>
    </GallerySection>
  </div>
</template>
