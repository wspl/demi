<script setup lang="ts">
// The third screen: what is the user's own and where it reaches: their
// data on their server, every device they use, the links they share, their
// team and their keys, a card each.
import { CheckIcon, CopyIcon, DatabaseIcon, KeyRoundIcon, LinkIcon, LockIcon, MailIcon, MessageSquareIcon, PlusIcon, ServerIcon } from '@lucide/vue'
import OtterCard from '@/components/OtterCard.vue'
import otterData from '@/assets/yours/data.webp'
import otterDevices from '@/assets/yours/devices.webp'
import otterKeys from '@/assets/yours/keys.webp'
import otterShare from '@/assets/yours/share.webp'
import otterTeam from '@/assets/yours/team.webp'

const conversations = [
  { title: 'Plan the team offsite', when: '2 min ago' },
  { title: 'Reorder the office supplies', when: '1 h ago' },
  { title: 'Fix the negative cart total', when: 'Yesterday' },
  { title: 'Q3 budget questions', when: 'Mon' },
]
const users = [
  { name: 'alex', email: 'alex@example.com', role: 'Admin' },
  { name: 'sam', email: 'sam@example.com', role: 'Member' },
  { name: 'priya', email: 'priya@example.com', role: 'Member' },
]
const credentials = [
  { name: 'Example API', icon: KeyRoundIcon },
  { name: 'Mail account', icon: MailIcon },
  { name: 'Database', icon: DatabaseIcon },
]
</script>

<template>
  <section class="yours screen">
    <div class="heading">
      <h2 class="section-title">Your server, your data.</h2>
      <p class="section-lede">
        Conversations and files stay on your server. Open them on any device,<br />
        share them with a link, and invite your team.
      </p>
    </div>

    <div class="bento">
      <OtterCard class="third" :icon="otterData" title="Your data" text="Conversations and files stay on your server. Change AI providers and keep your history.">
        <div class="panel">
          <div class="panel-head">
            <span class="server"><ServerIcon :size="15" />demi.example.com</span>
            <span class="stored"><i />On your server</span>
          </div>
          <div v-for="conversation in conversations" :key="conversation.title" class="row">
            <MessageSquareIcon :size="15" class="row-icon" />
            <span class="grow">{{ conversation.title }}</span>
            <small>{{ conversation.when }}</small>
          </div>
        </div>
      </OtterCard>

      <OtterCard class="third" :icon="otterDevices" title="Any device" text="Use it in any browser, or add it to your Home Screen. It adapts to phones, tablets and foldables.">
        <div class="devices">
          <figure>
            <div class="device iphone">
              <span class="island" />
              <span class="title-bar">Plan the team offsite</span>
              <span class="bubble mine">Find a venue for 12 near the coast.</span>
              <span class="lines"><i /><i /><i class="short" /></span>
              <span class="box" />
            </div>
            <figcaption>iPhone</figcaption>
          </figure>
          <figure>
            <div class="device duo">
              <span class="crease" />
              <span class="side">
                <span v-for="(conversation, index) in conversations" :key="conversation.title" class="side-row" :class="{ current: index === 0 }">{{ conversation.title }}</span>
              </span>
              <span class="chat">
                <span class="title-bar">Plan the team offsite</span>
                <span class="bubble mine">Find a venue for 12 near the coast.</span>
                <span class="lines"><i /><i /><i /><i class="short" /></span>
                <span class="box" />
              </span>
            </div>
            <figcaption>iPhone Duo</figcaption>
          </figure>
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

      <OtterCard class="half" :icon="otterTeam" title="Teams" text="Each member gets their own account and sandbox. Admins manage everyone in one place.">
        <div class="panel">
          <div class="panel-head">
            <span>Users</span>
            <span class="small-button"><PlusIcon :size="14" />Add User</span>
          </div>
          <div v-for="user in users" :key="user.name" class="row">
            <span class="avatar">{{ user.name[0]!.toUpperCase() }}</span>
            <span class="grow"><strong>{{ user.name }}</strong><small>{{ user.email }}</small></span>
            <span class="role">{{ user.role }}</span>
            <span class="sandbox"><i />Own sandbox</span>
          </div>
        </div>
      </OtterCard>

      <OtterCard class="half" :icon="otterKeys" title="Secrets" text="When Demi needs a password or an API key, it asks in a secure form. Secrets never appear in the conversation.">
        <div class="panel secrets">
          <div class="list">
            <div v-for="(credential, index) in credentials" :key="credential.name" class="list-row" :class="{ current: index === 0 }">
              <component :is="credential.icon" :size="15" />{{ credential.name }}
            </div>
          </div>
          <div class="detail">
            <strong>Example API</strong>
            <small>Demi needs a key to read your orders.</small>
            <span class="field"><LockIcon :size="13" />••••••••••••••••</span>
            <span class="dark-button">Provide</span>
            <small class="note"><LockIcon :size="12" />Never shown in the conversation.</small>
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

.third {
  grid-column: span 2;
}

.half {
  grid-column: span 3;
}

.panel {
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  font-size: 0.8125rem;
}

.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.75rem 0.5rem 0.875rem;
  border-bottom: 1px solid var(--line);
  font-size: 0.875rem;
}

.small-button {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.25rem 0.625rem;
  border: 1px solid var(--line-strong);
  border-radius: 0.5rem;
  font-size: 0.8125rem;
  font-weight: 500;
}

.row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.5rem 0.875rem;
  border-bottom: 1px solid var(--line);
}

.row:last-child {
  border-bottom: 0;
}

.row-icon {
  flex: none;
  color: var(--ink);
}

.grow {
  flex: 1;
  min-width: 0;
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


























.server,
.stored,
.sandbox {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
}

.stored,
.sandbox {
  color: var(--ink-soft);
  font-size: 0.75rem;
}

.stored i,
.sandbox i {
  width: 0.4375rem;
  height: 0.4375rem;
  border-radius: 50%;
  background: #5f9a6a;
}

.devices {
  display: flex;
  gap: 0.75rem;
  align-items: flex-end;
  justify-content: center;
  padding-top: 0.25rem;
}

figure {
  display: flex;
  margin: 0;
  flex-direction: column;
  align-items: center;
  gap: 0.375rem;
}

figcaption {
  color: var(--ink-faint);
  font-size: 0.75rem;
}

/* The screens keep their devices' shapes: an iPhone's 393 by 852 points, and iPhone Duo open, 2670 by 1878 pixels. */
.device {
  position: relative;
  display: flex;
  border: 0.3125rem solid #1c1918;
  background: var(--paper);
  font-size: 0.5625rem;
  line-height: 1.3;
}

.iphone {
  flex-direction: column;
  gap: 0.375rem;
  width: 5.25rem;
  aspect-ratio: 393 / 852;
  padding: 1.25rem 0.375rem 0.375rem;
  border-radius: 1rem;
}

.duo {
  width: 15rem;
  aspect-ratio: 2670 / 1878;
  border-radius: 0.875rem;
  overflow: hidden;
}

.island {
  position: absolute;
  top: 0.3125rem;
  left: 50%;
  width: 1.5rem;
  height: 0.4375rem;
  border-radius: 1rem;
  background: #1c1918;
  transform: translateX(-50%);
}

/* The fold, faint down the middle of the open screen. */
.crease {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 50%;
  width: 0.75rem;
  background: linear-gradient(90deg, transparent, rgb(60 40 25 / 7%), transparent);
  transform: translateX(-50%);
  pointer-events: none;
}

.side {
  display: flex;
  flex-direction: column;
  gap: 0.1875rem;
  width: 34%;
  padding: 0.5rem 0.3125rem;
  border-right: 1px solid var(--line);
  background: #f4efe8;
}

.side-row {
  overflow: hidden;
  padding: 0.1875rem 0.3125rem;
  border-radius: 0.3125rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.side-row.current {
  background: #e9e1d6;
}

.chat {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 0.375rem;
  min-width: 0;
  padding: 0.5rem 0.625rem 0.4375rem;
}

.title-bar {
  overflow: hidden;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bubble.mine {
  align-self: flex-end;
  max-width: 85%;
  padding: 0.25rem 0.375rem;
  border-radius: 0.375rem;
  background: #f1ebe3;
}

.lines {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.lines i {
  height: 0.25rem;
  border-radius: 1rem;
  background: #e6ded3;
}

.lines i.short {
  width: 60%;
}

.box {
  margin-top: auto;
  height: 0.875rem;
  border: 1px solid var(--line-strong);
  border-radius: 0.375rem;
}

.share {
  display: flex;
  flex-direction: column;
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

.avatar {
  display: grid;
  flex: none;
  place-items: center;
  width: 1.75rem;
  height: 1.75rem;
  border-radius: 50%;
  background: #eadfd2;
  font-weight: 600;
}

.role {
  width: 4.5rem;
  color: var(--ink-soft);
}

.secrets {
  display: grid;
  grid-template-columns: 40% 1fr;
}

.list {
  padding: 0.5rem;
  border-right: 1px solid var(--line);
}

.list-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem 0.5rem;
  border-radius: 0.5rem;
}

.list-row.current {
  background: #f1ebe3;
}

.detail {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
  padding: 0.75rem 0.875rem;
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

.note {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  margin-top: 0.25rem;
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

@media (max-width: 560px) {
  .secrets {
    grid-template-columns: minmax(0, 1fr);
  }

  .list {
    display: flex;
    flex-wrap: wrap;
    border-right: 0;
    border-bottom: 1px solid var(--line);
  }
}
</style>
