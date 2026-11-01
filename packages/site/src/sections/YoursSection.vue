<script setup lang="ts">
// The third screen: what is the user's own: the server Demi runs on, the
// workspaces that keep the parts of their life apart, the links they share,
// their data, which reaches no one but the AI providers they choose, and
// their team, a card each, each drawn its own way.
import { ref } from 'vue'
import { BriefcaseIcon, CheckIcon, CopyIcon, HouseIcon, LinkIcon, MessageSquareIcon, ServerIcon, SproutIcon, UsersIcon } from '@lucide/vue'
import OtterCard from '@/components/OtterCard.vue'
import { useOnScreen } from '@/onScreen'
import cloudSage from '@/assets/yours/cloud-sage.webp'
import cloudSlate from '@/assets/yours/cloud-slate.webp'
import cloudTerracotta from '@/assets/yours/cloud-terracotta.webp'
import otterData from '@/assets/yours/data.webp'
import otterSelfHost from '@/assets/yours/selfhost.webp'
import otterShare from '@/assets/yours/share.webp'
import otterSpaces from '@/assets/yours/spaces.webp'
import otterTeam from '@/assets/yours/team.webp'

/** The workspaces, the front one open on its conversations. */
const workspaces = [
  { name: 'Work', icon: BriefcaseIcon, rows: ['Plan the team offsite', 'Fix the negative cart total', 'Q3 budget questions', 'Draft the release notes'] },
  { name: 'Personal', icon: HouseIcon, rows: [] },
  { name: 'Harbor Goods', icon: UsersIcon, rows: [] },
  { name: 'Side project', icon: SproutIcon, rows: [] },
]
/** How much the server holds, month by month, the latest last. */
const MONTHS = [22, 30, 28, 41, 47, 55, 52, 66, 74, 81]
const members = [
  { name: 'alex', role: 'Admin', tint: '#eadfd2', sandbox: cloudTerracotta },
  { name: 'sam', role: 'Member', tint: '#dfe6dc', sandbox: cloudSage },
  { name: 'priya', role: 'Member', tint: '#e2e5ec', sandbox: cloudSlate },
]

const terminal = ref<HTMLElement | null>(null)
const terminalOnScreen = useOnScreen(terminal)
</script>

<template>
  <section class="yours screen">
    <div class="heading">
      <h2 class="section-title">Your server, your data.</h2>
      <p class="section-lede">
        Demi runs on a server of your own. Keep work and life apart,<br />
        share with a link, and invite your team.
      </p>
    </div>

    <div class="bento">
      <OtterCard class="third" :icon="otterSelfHost" title="Self-hosted" text="One server of yours runs it all: the conversations, the agents and each member's sandbox. Install it with one command and update it from Settings.">
        <div ref="terminal" class="terminal" :class="{ paused: !terminalOnScreen }">
          <p><span class="prompt">$</span> <span class="cmd">curl</span> <span class="flag">-fsSL</span> <span class="url">https://demi.sh/install.sh</span> <span class="op">|</span> <span class="cmd">sudo bash</span></p>
          <p class="faint">==&gt; Installing Demi 0.4.2 (Beta)</p>
          <p class="ok"><CheckIcon :size="12" /> Cloud sandboxes ready</p>
          <p class="ok"><CheckIcon :size="12" /> HTTPS for <span class="url">demi.example.com</span></p>
          <p class="ok"><CheckIcon :size="12" /> Running, updates from Settings</p>
          <p class="faint">Open <span class="url">https://demi.example.com</span> to create</p>
          <p class="faint">the admin account.</p>
          <p><span class="prompt">$</span> <i class="cursor" /></p>
        </div>
      </OtterCard>

      <OtterCard class="third" :icon="otterSpaces" title="Workspaces" text="Keep work, personal and client projects apart. Each workspace has its own conversations, projects and schedules.">
        <div class="spaces">
          <div class="tabs">
            <span v-for="(space, index) in workspaces" :key="space.name" class="tab" :class="{ current: index === 0 }">
              <component :is="space.icon" :size="13" />{{ space.name }}
            </span>
          </div>
          <div class="sheet">
            <span v-for="row in workspaces[0]!.rows" :key="row" class="sheet-row"><MessageSquareIcon :size="13" />{{ row }}</span>
          </div>
        </div>
      </OtterCard>

      <OtterCard class="third" :icon="otterShare" title="Sharing" text="Share a conversation, or a single answer, as a read-only link.">
        <div class="panel share">
          <strong>Share Conversation</strong>
          <small>Anyone with the link can read it.</small>
          <div class="link-row">
            <span class="field"><LinkIcon :size="13" />demi.example.com/s/4f9a2e7c</span>
            <span class="dark-button"><CopyIcon :size="13" />Copy Link</span>
          </div>
          <div class="option current"><CheckIcon :size="14" />The whole conversation</div>
          <div class="option">This answer only</div>
        </div>
      </OtterCard>

      <OtterCard class="half" :icon="otterData" title="Your data" text="Conversations and files stay on your server, and nothing goes through us: it talks straight to the AI providers you choose. Change providers and keep your history.">
        <div class="stats">
          <div class="stat-head"><ServerIcon :size="14" />demi.example.com</div>
          <div class="figures">
            <span><b>1,284</b><small>conversations</small></span>
            <span><b>3.2 GB</b><small>files</small></span>
            <span><b>4</b><small>providers used</small></span>
          </div>
          <div class="months">
            <i v-for="(height, n) in MONTHS" :key="n" :style="{ height: `${height}%` }" />
          </div>
          <small class="since">All on your disk, since March. No Demi cloud in between.</small>
        </div>
      </OtterCard>

      <OtterCard class="half" :icon="otterTeam" title="Teams" text="Each member gets their own account and sandbox. Admins manage everyone in one place.">
        <div class="members">
          <div v-for="member in members" :key="member.name" class="member">
            <span class="avatar" :style="{ background: member.tint }">{{ member.name[0]!.toUpperCase() }}</span>
            <strong>{{ member.name }}</strong>
            <span class="role" :class="{ admin: member.role === 'Admin' }">{{ member.role }}</span>
            <span class="box"><img loading="lazy" :src="member.sandbox" alt="" />Own sandbox</span>
          </div>
        </div>
      </OtterCard>
    </div>
  </section>
</template>

<style scoped>
.heading {
  margin-bottom: clamp(1.25rem, 3.4vh, 2rem);
  text-align: center;
}

.section-lede {
  margin-right: auto;
  margin-left: auto;
}

.bento {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: 1rem;
  max-width: 80rem;
  margin: 0 auto;
  width: 100%;
}

/*
 * Each card takes two rows of the grid, its head and its panel, on the
 * grid's own rows, so the cards side by side start their panels on one line
 * and end them on another, however long their text runs.
 */
.third,
.half {
  display: grid;
  grid-row: span 2;
  grid-template-rows: subgrid;
  row-gap: 0;
}

.third {
  grid-column: span 2;
}

.half {
  grid-column: span 3;
}

.panel {
  flex: 1;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  font-size: 0.8125rem;
}

strong {
  display: block;
  font-weight: 550;
}

small {
  display: block;
  color: var(--ink-faint);
  font-size: 0.75rem;
}

/* Self-hosted: the install, as a light terminal prints it. */
.terminal {
  flex: 1;
  padding: 0.75rem 0.875rem;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: #f3eee7;
  font: 400 0.75rem/1.65 var(--mono);
  color: var(--ink);
}

.terminal p {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  margin: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.prompt,
.terminal .faint {
  color: var(--ink-faint);
}

/* The command and the output in the colors a shell's highlighting gives them. */
.cmd {
  color: var(--ink);
  font-weight: 600;
}

.flag {
  color: #8a6235;
}

.url {
  color: var(--accent);
}

.op {
  color: var(--ink-faint);
}

.cursor {
  display: inline-block;
  width: 0.45em;
  height: 1.1em;
  background: var(--ink-soft);
  animation: caret 1s steps(1) infinite;
}

.paused .cursor {
  animation-play-state: paused;
}

@keyframes caret {
  50% {
    opacity: 0;
  }
}

.terminal .ok svg {
  flex: none;
  color: #5f9a6a;
}

/* Workspaces: a tab each, the front one open on its conversations. */
.spaces {
  position: relative;
  display: flex;
  flex: 1;
  flex-direction: column;
}

.tabs {
  display: flex;
  gap: 0.25rem;
  overflow: hidden;
  font-size: 0.75rem;
}

.tab {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  padding: 0.3rem 0.55rem;
  border: 1px solid transparent;
  border-bottom: 0;
  border-radius: 0.5rem 0.5rem 0 0;
  color: var(--ink-faint);
  white-space: nowrap;
}

.tab.current {
  position: relative;
  z-index: 3;
  border-color: var(--line);
  background: var(--paper);
  color: var(--ink);
  margin-bottom: -1px;
}

.sheet {
  position: relative;
  z-index: 2;
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 0.15rem;
  padding: 0.5rem;
  border: 1px solid var(--line);
  border-radius: 0 0.75rem 0.75rem;
  background: var(--paper);
  font-size: 0.8125rem;
}

.sheet-row {
  display: flex;
  flex: 1;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.4rem;
  border-radius: 0.4rem;
  color: var(--ink);
}

.sheet-row:first-child {
  background: #f6f1ea;
}

.sheet-row svg {
  color: var(--ink-faint);
}

/* Teams: each member a card of their own, with a sandbox of their own. */
.members {
  display: grid;
  flex: 1;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0.625rem;
}

.member {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.35rem;
  padding: 0.75rem 0.625rem;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  font-size: 0.8125rem;
}

.avatar {
  display: grid;
  place-items: center;
  width: 2.25rem;
  height: 2.25rem;
  border-radius: 50%;
  font-weight: 600;
}

.role {
  display: inline-flex;
  align-items: center;
  box-sizing: border-box;
  height: 1.25rem;
  /* A word without descenders sits high in its line: room above it centers its ink. */
  padding: 2px 0.55rem 0;
  border-radius: 999px;
  background: #f1ebe3;
  color: var(--ink-soft);
  font-size: 0.7rem;
  line-height: 1;
}

.role.admin {
  background: #e8dccd;
  color: var(--ink);
}

.box {
  display: inline-flex;
  align-self: stretch;
  align-items: center;
  justify-content: center;
  gap: 0.35rem;
  margin-top: 0.35rem;
  padding: 0.3rem 0.5rem;
  border: 1px dashed var(--line-strong);
  border-radius: 0.5rem;
  color: var(--ink-soft);
  font: 400 0.7rem/1.2 var(--mono);
}

.box img {
  width: 1.25rem;
  height: 0.875rem;
}

/* Your data: how much of it the server holds, growing month by month. */
.stats {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 0.6rem;
  padding: 0.75rem 0.875rem 1rem;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  font-size: 0.8125rem;
}

.stat-head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.figures {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
}

.figures b {
  display: block;
  font: 600 1.35rem/1.1 var(--serif);
}

.months {
  display: flex;
  flex: 1;
  align-items: flex-end;
  gap: 0.3rem;
  min-height: 2.2rem;
}

.months i {
  flex: 1;
  border-radius: 0.2rem 0.2rem 0 0;
  background: #e8dccd;
}

.months i:last-child {
  background: var(--accent);
}

.share {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 0.25rem;
  padding: 0.75rem 0.875rem;
}

.link-row {
  display: flex;
  gap: 0.5rem;
  margin: 0.375rem 0 0.25rem;
}

.link-row .field {
  flex: 1;
  min-width: 0;
  margin-top: 0;
}

.link-row .dark-button {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  margin-top: 0;
  white-space: nowrap;
}

.option {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.3125rem 0.5rem 0.3125rem 1.875rem;
  border-radius: 0.5rem;
}

.option.current {
  padding-left: 0.5rem;
  background: #f1ebe3;
}

.field {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  overflow: hidden;
  white-space: nowrap;
  margin-top: 0.25rem;
  padding: 0.3125rem 0.625rem;
  border: 1px solid var(--line);
  border-radius: 0.5rem;
  color: var(--ink-faint);
}

.dark-button {
  align-self: flex-start;
  margin-top: 0.25rem;
  padding: 0.3125rem 0.875rem;
  border-radius: 0.5rem;
  background: var(--button);
  color: #fbf7f0;
  font-weight: 500;
}

@media (max-width: 900px) {
  .third,
  .half {
    grid-column: span 3;
  }

  .half:last-child {
    grid-column: span 6;
  }
}

@media (max-width: 600px) {
  .third,
  .half {
    grid-column: span 6;
  }
}

</style>
