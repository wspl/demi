<script setup lang="ts">
import { computed, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import ExternalLink from '@demicodes/web-ui/ui/ExternalLink.vue'
import Segmented, { type SegmentedOption } from '@demicodes/web-ui/ui/Segmented.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import {
  TITLE_LOWERCASE_WORDS,
  applyStyle,
  styleProblems,
  type TextStyle,
} from '@demicodes/web-ui/ui/ui-text'
import GallerySection from '../components/GallerySection.vue'

/**
 * The rule for UI text: macOS capitalization, as Apple's Human Interface
 * Guidelines and Apple Style Guide prescribe it. `ui-text.ts` is the rule in
 * code; the UI text check (`scripts/ui-text-check.ts`) applies it to every
 * literal whose place declares a style.
 */

interface Source {
  label: string
  href?: string
}

interface ElementRule {
  kind: string
  /** The web-ui place that declares the style, as the check reports it. */
  where: string
  /** `content` is text Demi shows but does not write: it stays as written. */
  style: TextStyle | 'content'
  right: readonly string[]
  wrong: readonly string[]
  /** Apple's word for it, or how Demi maps a web element Apple does not name. */
  source: Source
}

const HIG = 'https://developer.apple.com/design/human-interface-guidelines'
const STYLE_GUIDE_CAPITALIZATION = 'https://support.apple.com/guide/applestyleguide/c-apsgb744e4a3/web'
const MACOS_HIG_2016 = 'https://web.archive.org/web/20161225025205/https://developer.apple.com/library/content/documentation/UserExperience/Conceptual/OSXHIGuidelines/TerminologyWording.html'
const SYSTEM_SETTINGS = 'macOS 26 System Settings'

const rules: readonly ElementRule[] = [
  {
    kind: 'Menu item',
    where: 'MenuItem label, a Dropdown’s options',
    style: 'title',
    right: ['New Conversation Here', 'Use as Primary Environment…'],
    wrong: ['New conversation here'],
    source: { label: 'HIG: Menus', href: `${HIG}/menus` },
  },
  {
    kind: 'Menu section heading',
    where: 'MenuGroup label',
    style: 'title',
    right: ['Attached Hosts', 'Exposed URLs'],
    wrong: ['Attached hosts'],
    source: { label: 'macOS HIG 2016: list headings', href: MACOS_HIG_2016 },
  },
  {
    kind: 'Button',
    where: 'Button content, a dialog’s buttons, a notice’s action',
    style: 'title',
    right: ['Add Source…', 'Try Again', 'Sign In', 'Don’t Save'],
    wrong: ['Add source', 'Sign in'],
    source: { label: 'HIG: Buttons', href: `${HIG}/buttons` },
  },
  {
    kind: 'Segmented control',
    where: 'Segmented options',
    style: 'title',
    right: ['Diff', 'Preview', 'One at a Time'],
    wrong: ['One at a time'],
    source: { label: 'HIG: Segmented controls', href: `${HIG}/segmented-controls` },
  },
  {
    kind: 'Tab',
    where: 'A work panel tab’s title, the gallery’s view tabs',
    style: 'title',
    right: ['File Browser', 'Messages & Uploads'],
    wrong: ['Messages & uploads'],
    source: { label: 'HIG: Tab views', href: `${HIG}/tab-views` },
  },
  {
    kind: 'Sidebar and settings navigation',
    where: 'SidebarNavItem label, a settings section’s name',
    style: 'title',
    right: ['Models & Providers', 'MCP Servers'],
    wrong: ['Models & providers'],
    source: { label: `${SYSTEM_SETTINGS}: Desktop & Dock, Login Items` },
  },
  {
    kind: 'Section or group title',
    where: 'SettingsPage title, SettingsGroup title',
    style: 'title',
    right: ['Data & Privacy', 'Danger Zone', 'Command-Line Tool'],
    wrong: ['Danger zone', 'Command-line tool'],
    source: { label: `${SYSTEM_SETTINGS}: Input Sources, Function Keys` },
  },
  {
    kind: 'Column heading',
    where: 'A table’s th, a list’s column names',
    style: 'title',
    right: ['Date Modified'],
    wrong: ['Date modified'],
    source: { label: 'HIG: Lists and tables', href: `${HIG}/lists-and-tables` },
  },
  {
    kind: 'Dialog title',
    where: 'Dialog label and the heading it shows',
    style: 'headline',
    right: ['Add Device', 'Change Password'],
    wrong: ['Add device'],
    source: { label: 'macOS HIG 2016: the command’s name, without its ellipsis', href: MACOS_HIG_2016 },
  },
  {
    kind: 'Alert or toast title',
    where: 'reportError and showToast title, a confirmation’s heading',
    style: 'headline',
    right: ['Could Not Revoke Device', 'Remove this project?'],
    wrong: ['Could not revoke device', 'Remove This Project?'],
    source: { label: 'HIG: Alerts', href: `${HIG}/alerts` },
  },
  {
    kind: 'Empty state',
    where: 'Menu and ChangeTree emptyText, an empty list',
    style: 'headline',
    right: ['No Hosts Found', 'This folder is empty.'],
    wrong: ['No hosts found'],
    source: { label: `${SYSTEM_SETTINGS}: No Accounts, No external volumes are connected.` },
  },
  {
    kind: 'Settings row label',
    where: 'SettingsRow label',
    style: 'sentence',
    right: ['Pairing code', 'Your Cloud environment', 'Key repeat rate'],
    wrong: ['Pairing Code'],
    source: { label: `${SYSTEM_SETTINGS}: Sidebar icon size, Key repeat rate` },
  },
  {
    kind: 'Switch, checkbox or choice',
    where: 'Switch, Checkbox and ChoiceCards label',
    style: 'sentence',
    right: ['Show scroll bars', 'Automatically hide and show the Dock'],
    wrong: ['Show Scroll Bars'],
    source: { label: 'macOS HIG 2016: options that are not strictly labels', href: MACOS_HIG_2016 },
  },
  {
    kind: 'Field label',
    where: 'A form’s label element',
    style: 'sentence',
    right: ['Project name', 'Full name'],
    wrong: ['Project Name'],
    source: { label: `${SYSTEM_SETTINGS}: Users & Groups` },
  },
  {
    kind: 'Placeholder',
    where: 'TextInput placeholder, a menu’s filter',
    style: 'placeholder',
    right: ['Search hosts', 'name@example.com'],
    wrong: ['Search Hosts.'],
    source: { label: 'Safari: Search or enter website name' },
  },
  {
    kind: 'Tooltip',
    where: 'Tooltip content, disabledReason, a title attribute',
    style: 'sentence',
    right: ['Open in a browser tab', 'Add a DNS server address'],
    wrong: ['Open in a Browser Tab'],
    source: { label: 'HIG: Offering help', href: `${HIG}/offering-help` },
  },
  {
    kind: 'Message',
    where: 'A description, a toast’s message, InlineError, RegionStatus and ErrorNotice label',
    style: 'sentence',
    right: ['Couldn’t load this conversation.', 'The two passwords differ.'],
    wrong: ['Couldn’t Load This Conversation'],
    source: { label: 'HIG: Alerts, informative text', href: `${HIG}/alerts` },
  },
  {
    kind: 'Status',
    where: 'AsyncRegion label, a settings row’s status tag',
    style: 'sentence',
    right: ['Loading devices…', 'Update available'],
    wrong: ['Update Available'],
    source: { label: 'macOS HIG 2016: Checking for new software…', href: MACOS_HIG_2016 },
  },
  {
    kind: 'Content',
    where: 'A conversation’s or project’s name, a file name, a model’s id, what a model wrote',
    style: 'content',
    right: ['claude-sonnet', 'fix login bug'],
    wrong: [],
    source: { label: 'Apple Style Guide: names exactly as they appear', href: STYLE_GUIDE_CAPITALIZATION },
  },
]

const STYLE_NAMES: Record<ElementRule['style'], string> = {
  title: 'Title',
  sentence: 'Sentence',
  headline: 'Title as a fragment, sentence as a sentence',
  placeholder: 'Sentence, or an example value',
  content: 'As written',
}

const styleOptions: readonly SegmentedOption<TextStyle>[] = [
  { value: 'title', label: 'Title' },
  { value: 'sentence', label: 'Sentence' },
  { value: 'headline', label: 'Headline' },
  { value: 'placeholder', label: 'Placeholder' },
]

const wordRules = [
  {
    title: 'Capitalize',
    items: [
      'The first and the last word, whatever they are: Go To…, Used For',
      'Nouns, pronouns, verbs, adjectives and adverbs, however short: Is, Are, Be, It, This, Then',
      'Subordinating conjunctions: What to Do If Your iPhone Is Lost',
      'Prepositions of five letters or more: About, Between, Through, Without',
      'A preposition that belongs to a phrasal verb: Sign In, Set Up, Turn Off, Log In to the Server',
      'The word after a hyphen: Command-Line Tool, High-Level Events, 64-Bit; except Built-in and Plug-in',
      'The first word after a colon',
    ],
  },
  {
    title: 'Keep Lowercase',
    items: [
      `Articles: ${TITLE_LOWERCASE_WORDS.articles.join(', ')}`,
      `Coordinating conjunctions: ${TITLE_LOWERCASE_WORDS.coordinatingConjunctions.join(', ')}`,
      'To in an infinitive, and as in any role: How to Start, Export a Document as a PDF',
      'Prepositions of four letters or fewer: at, by, for, from, in, into, of, off, on, onto, out, over, to, up, with',
    ],
  },
  {
    title: 'In Both Styles',
    items: [
      'A name keeps its spelling: iPhone, macOS, MCP, Claude Code, Cloud',
      'An ellipsis (…, one character) ends a command that needs more input before it acts: it opens a dialog to fill in, or always asks to confirm. A command that acts at once, or opens a panel to look at, has none.',
      'A title has no ending punctuation and no colon. A sentence ends with its period or question mark, except a tooltip that is a fragment.',
      'A sentence names another element in that element’s capitals, without its ellipsis: Choose Add Source to add one.',
    ],
  },
]

const sources: readonly Required<Source>[] = [
  { label: 'Human Interface Guidelines: Writing', href: `${HIG}/writing` },
  { label: 'Human Interface Guidelines: Menus', href: `${HIG}/menus` },
  { label: 'Human Interface Guidelines: The menu bar', href: `${HIG}/the-menu-bar` },
  { label: 'Human Interface Guidelines: Buttons', href: `${HIG}/buttons` },
  { label: 'Human Interface Guidelines: Alerts', href: `${HIG}/alerts` },
  { label: 'Human Interface Guidelines: Tab views', href: `${HIG}/tab-views` },
  { label: 'Human Interface Guidelines: Segmented controls', href: `${HIG}/segmented-controls` },
  { label: 'Human Interface Guidelines: Lists and tables', href: `${HIG}/lists-and-tables` },
  { label: 'Human Interface Guidelines: Panels', href: `${HIG}/panels` },
  { label: 'Human Interface Guidelines: Notifications', href: `${HIG}/notifications` },
  { label: 'Human Interface Guidelines: Offering help', href: `${HIG}/offering-help` },
  { label: 'Apple Style Guide: capitalization', href: STYLE_GUIDE_CAPITALIZATION },
  { label: 'Apple Style Guide: ellipsis', href: 'https://support.apple.com/guide/applestyleguide/e-apsg076a7313/web' },
  { label: 'Apple Style Guide: help tag', href: 'https://support.apple.com/guide/applestyleguide/h-apsg9dac5903/web' },
  { label: 'OS X Human Interface Guidelines (2016): Terminology and wording', href: MACOS_HIG_2016 },
]

const text = ref('Add source...')
const style = ref<TextStyle>('title')
const tester = ref<HTMLElement | null>(null)

const problems = computed(() => styleProblems(text.value, style.value))
const fixed = computed(() => applyStyle(text.value, style.value))

/** Loads an example into the tester, which then says what, if anything, to change. */
function tryExample(example: string, exampleStyle: ElementRule['style']): void {
  if (exampleStyle === 'content')
    return
  text.value = example
  style.value = exampleStyle
  tester.value?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
}

function applyFixes(): void {
  text.value = fixed.value
}
</script>

<template>
  <div class="space-y-8">
    <div class="max-w-3xl space-y-3 text-[13px] leading-6 text-fg-body">
      <p>
        Demi writes its UI text the way macOS does. A menu item, a button or a
        title is in title style: <span class="text-fg-emphasis">Add Source…</span>.
        A switch, a settings row, a tooltip or a message is in sentence style:
        <span class="text-fg-emphasis">Show scroll bars</span>. Where Apple names
        no style for an element Demi has, the table below maps it to the macOS
        element it works like, and says so.
      </p>
      <p class="text-fg-muted">
        A prop that carries UI text declares its style by its type (TitleText,
        SentenceText, HeadlineText or PlaceholderText), and the UI text check
        in bun run test applies this rule to every literal written into one.
        Documentation prose stays in the Google style.
      </p>
    </div>

    <GallerySection title="Try a Text" note="Pick a style, type a text, and see what the rule changes. A click on an example below loads it here.">
      <div ref="tester" class="max-w-3xl space-y-3 rounded-xl border border-line bg-surface p-4">
        <Segmented v-model="style" :options="styleOptions" size="sm" />
        <TextInput v-model="text" placeholder="A label, title or message" />
        <div class="min-h-10 text-[13px] leading-5" aria-live="polite">
          <p v-if="text.trim() === ''" class="text-fg-muted">Type a text to check it.</p>
          <p v-else-if="problems.length === 0" class="text-fg-body">Follows the rule.</p>
          <ul v-else class="space-y-1 text-fg-body">
            <li v-for="problem in problems" :key="`${problem.word}-${problem.want}`">
              <span class="font-mono text-fg-emphasis">{{ problem.word }}</span>
              becomes
              <span class="font-mono text-fg-emphasis">{{ problem.want || 'nothing' }}</span>
            </li>
            <li v-if="style === 'headline'" class="text-fg-muted">Or write it as a complete sentence with its ending punctuation.</li>
          </ul>
        </div>
        <div class="flex gap-2">
          <Button size="sm" :disabled="problems.length === 0" disabled-reason="Nothing to change" @click="applyFixes">Apply Fixes</Button>
          <Button size="sm" variant="ghost" :disabled="text === ''" disabled-reason="The field is empty" @click="text = ''">Clear</Button>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Which Style Each Element Takes">
      <div class="overflow-x-auto rounded-xl border border-line">
        <table class="w-full min-w-[56rem] text-left text-[12px] leading-4">
          <thead class="bg-surface-raised text-fg-subtle">
            <tr>
              <th class="px-3 py-2 font-medium">Element</th>
              <th class="px-3 py-2 font-medium">In Demi</th>
              <th class="px-3 py-2 font-medium">Style</th>
              <th class="px-3 py-2 font-medium">Right</th>
              <th class="px-3 py-2 font-medium">Wrong</th>
              <th class="px-3 py-2 font-medium">Source</th>
            </tr>
          </thead>
          <tbody class="text-fg-body">
            <tr v-for="rule in rules" :key="rule.kind" class="border-t border-line align-top">
              <td class="px-3 py-2 text-fg-emphasis">{{ rule.kind }}</td>
              <td class="px-3 py-2 text-fg-muted">{{ rule.where }}</td>
              <td class="px-3 py-2">{{ STYLE_NAMES[rule.style] }}</td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap gap-1">
                  <template v-for="example in rule.right" :key="example">
                    <button
                      v-if="rule.style !== 'content'"
                      type="button"
                      class="rounded px-1.5 py-0.5 text-left text-fg-body transition-colors duration-200 ease-out bg-overlay/6 hover:bg-hover"
                      @click="tryExample(example, rule.style)"
                    >{{ example }}</button>
                    <span v-else class="rounded px-1.5 py-0.5 bg-overlay/6">{{ example }}</span>
                  </template>
                </div>
              </td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap gap-1">
                  <button
                    v-for="example in rule.wrong"
                    :key="example"
                    type="button"
                    class="rounded px-1.5 py-0.5 text-left text-fg-muted line-through transition-colors duration-200 ease-out hover:bg-hover"
                    @click="tryExample(example, rule.style)"
                  >{{ example }}</button>
                </div>
              </td>
              <td class="px-3 py-2">
                <ExternalLink v-if="rule.source.href" :href="rule.source.href">{{ rule.source.label }}</ExternalLink>
                <span v-else class="text-fg-muted">{{ rule.source.label }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </GallerySection>

    <GallerySection title="Title-Style Words" note="The Apple Style Guide’s rule for each word of a title.">
      <div class="grid max-w-5xl gap-4 md:grid-cols-3">
        <div v-for="group in wordRules" :key="group.title" class="space-y-2 rounded-xl border border-line p-3">
          <h3 class="text-[13px] font-medium text-fg-emphasis">{{ group.title }}</h3>
          <ul class="space-y-1.5 text-[12px] leading-5 text-fg-body">
            <li v-for="item in group.items" :key="item">{{ item }}</li>
          </ul>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Sources" note="Apple’s own guides. Where they are silent, the rows above follow what macOS 26 System Settings shows.">
      <ul class="max-w-3xl space-y-1">
        <li v-for="source in sources" :key="source.href">
          <ExternalLink :href="source.href">{{ source.label }}</ExternalLink>
        </li>
      </ul>
    </GallerySection>
  </div>
</template>
