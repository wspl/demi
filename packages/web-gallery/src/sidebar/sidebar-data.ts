import { GALLERY_MARKS } from '../fixtures/device-marks'
import type { SavedScope, SidebarAccount, SidebarConversation, SidebarProject } from '@demicodes/web-ui/sidebar/types'
import { productWould } from '../product-would'
import { isConversationActive } from '@demicodes/web-ui/agent/conversation-status'
import { ago } from '../fixtures/time'

const HOUR = 60 * 60 * 1000
const DAY = 24 * HOUR

export const demoAccount: SidebarAccount = {
  id: 'alex',
  name: 'Alex',
  email: 'alex@example.com',
}

/** An account with no name, so its email stands in: too long for the narrowest sidebar, not the default. */
export const emailOnlyAccount: SidebarAccount = {
  id: 'alex-chen',
  name: '',
  email: 'alex.chen@acme.dev',
}

/** A master account as setup leaves it, with no name and an email too long for the default sidebar. */
export const longEmailAccount: SidebarAccount = {
  id: 'someone-longname',
  name: '',
  email: 'someone.longname@protonmail.example',
}

export function demoProjects(): SidebarProject[] {
  return [
    {
      id: 'p-demi',
      // The same name runs on another device below: both rows name their device before the mark.
      name: 'demi',
      hostKind: 'device',
      device: { kind: 'paired', name: 'alex-mbp', state: 'online', mark: GALLERY_MARKS.mac },
      path: '/Users/alex/Projects/demi',
      // Its conversations share one list of saved messages, of three.
      savedScope: 'project',
      savedCount: 3,
    },
    {
      id: 'p-demi-mini',
      name: 'demi',
      hostKind: 'device',
      device: { kind: 'paired', name: 'mac-mini', state: 'online', mark: GALLERY_MARKS.macMini },
      path: '/Users/alex/Projects/demi',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-api-work',
      // Two of one name on one device: each row ends its name with the folder it is in.
      name: 'api',
      hostKind: 'device',
      device: { kind: 'paired', name: 'alex-mbp', state: 'online', mark: GALLERY_MARKS.mac },
      path: '/Users/alex/work/api',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-api-old',
      name: 'api',
      hostKind: 'device',
      device: { kind: 'paired', name: 'alex-mbp', state: 'online', mark: GALLERY_MARKS.mac },
      path: '/Users/alex/old/api',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-photo-album',
      // The only project of its name: the mark alone shows its device, whose name shows in its tip.
      name: 'photo-album',
      hostKind: 'device',
      device: { kind: 'paired', name: 'Alexs-MacBook-Pro.local', state: 'online', mark: GALLERY_MARKS.macPro },
      path: '/Users/alex/Projects/photo-album',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-assets',
      // A long name, cut at its end; the device is offline.
      name: 'renderfarm-render-pipeline',
      hostKind: 'device',
      device: { kind: 'paired', name: 'build-01', state: 'offline', mark: GALLERY_MARKS.ubuntu },
      path: '/srv/renderfarm-render-pipeline',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-dotfiles',
      // A long name on a long device's name, with a namesake on another device: the device's
      // name is cut at half of the text, the project's name in the rest; each shows whole on hover.
      name: 'dotfiles-and-machine-setup-scripts',
      hostKind: 'device',
      // Its runner replaces itself with the server's release, a quarter downloaded.
      device: { kind: 'paired', name: 'Alexs-MacBook-Pro.local', state: 'updating', downloadPercent: 26, mark: GALLERY_MARKS.macPro },
      path: '/Users/alex/dotfiles-and-machine-setup-scripts',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-dotfiles-build',
      name: 'dotfiles-and-machine-setup-scripts',
      hostKind: 'device',
      device: { kind: 'paired', name: 'build-01', state: 'offline', mark: GALLERY_MARKS.ubuntu },
      path: '/home/build/dotfiles-and-machine-setup-scripts',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-old',
      // Its device was removed since: the mark has no dot.
      name: 'old-experiments',
      hostKind: 'device',
      device: { kind: 'removed' },
      path: '/Users/alex/old-experiments',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-notebooks',
      name: 'notebooks',
      hostKind: 'cloud',
      path: '/home/demi/notebooks',
      savedScope: 'conversation',
      savedCount: 0,
    },
    {
      id: 'p-scratch',
      // No conversation runs in it, so the list does not show it.
      name: 'scratch',
      hostKind: 'device',
      device: { kind: 'paired', name: 'alex-mbp', state: 'online', mark: GALLERY_MARKS.mac },
      path: '/Users/alex/scratch',
      savedScope: 'conversation',
      savedCount: 0,
    },
  ]
}

export function demoConversations(): SidebarConversation[] {
  return [
    // Plain conversations: questions with no checkout behind them.
    {
      id: 'c-child-running',
      title: 'Parent finished · subagent still running',
      updatedAt: ago(60 * 1000),
      status: isConversationActive('idle', [{ phase: 'running' }]) ? 'active' : 'idle',
      projectId: null,
      pinned: false,
      unread: false,
    },
    {
      // Not sent yet, its draft holds text: the placeholder title, which
      // holds no words and reads New conversation in the page's language.
      id: 'c-untitled',
      title: null,
      updatedAt: ago(2 * 60 * 1000),
      status: 'idle',
      projectId: null,
      pinned: false,
      unread: false,
    },
    {
      id: 'c-shift',
      title: 'How does floating-ui limitShift differ from shift?',
      updatedAt: ago(25 * 60 * 1000),
      status: 'done',
      projectId: null,
      pinned: false,
      unread: true,
      // Runs on a paired device that is online: the row's end shows it.
      device: { kind: 'paired', name: 'alex-mbp', state: 'online', mark: GALLERY_MARKS.mac },
    },
    {
      id: 'c-skills',
      title: 'Set up the web design skill',
      updatedAt: ago(12 * 60 * 1000),
      status: 'done',
      projectId: null,
      pinned: false,
      unread: true,
      // A permission request waits: the yellow mark wins over the finished one.
      needsYou: true,
    },
    {
      id: 'c-wording',
      title: 'Draft the quarterly newsletter',
      updatedAt: ago(5 * HOUR),
      status: 'idle',
      projectId: null,
      // Pinned on an offline device: the device's mark stands left of the pin.
      pinned: true,
      unread: false,
      device: { kind: 'paired', name: 'build-01', state: 'offline', mark: GALLERY_MARKS.ubuntu },
    },
    {
      id: 'c-regex',
      title: 'Regex for fenced code lines',
      updatedAt: ago(2 * DAY),
      status: 'idle',
      projectId: null,
      pinned: false,
      unread: false,
      // Its device was removed since: the mark has no dot.
      device: { kind: 'removed' },
    },
    // demi
    {
      id: 'c-login',
      title: 'Login test after the session cookie rename',
      updatedAt: ago(4 * 60 * 1000),
      status: 'active',
      projectId: 'p-demi',
      pinned: true,
      unread: false,
      // Running, open, and a subagent asks: the yellow mark wins over the breathing one.
      needsYou: true,
      // In a linked worktree of the project: its mark joins the row's end, after the pin.
      worktree: 'login-test',
    },
    {
      id: 'c-cookie',
      title: 'Cookie helper refactor',
      updatedAt: ago(40 * 60 * 1000),
      status: 'done',
      projectId: 'p-demi',
      pinned: false,
      unread: true,
      // In a linked worktree of the project, by git's name: its mark alone ends the row.
      worktree: '2',
    },
    {
      id: 'c-gallery',
      title: 'Compare three laptop models',
      updatedAt: ago(3 * HOUR),
      status: 'idle',
      projectId: 'p-demi',
      pinned: false,
      unread: false
    },
    {
      id: 'c-rate',
      title: 'Rate limit retry policy',
      updatedAt: ago(6 * HOUR),
      status: 'error',
      projectId: 'p-demi',
      pinned: false,
      unread: true
    },
    {
      id: 'c-tabs',
      title: 'Fix the flaky login test',
      updatedAt: ago(DAY + 2 * HOUR),
      status: 'idle',
      projectId: 'p-demi',
      pinned: false,
      unread: false
    },
    {
      id: 'c-shell',
      title: 'Summarize the customer interviews',
      updatedAt: ago(3 * DAY),
      status: 'idle',
      projectId: 'p-demi',
      pinned: false,
      unread: false
    },
    // renderfarm
    {
      id: 'c-thumbs',
      title: 'Resize the product photos for the shop',
      updatedAt: ago(DAY + 9 * HOUR),
      status: 'aborted',
      projectId: 'p-assets',
      pinned: false,
      unread: true
    },
    {
      id: 'c-exif',
      title: 'Strip EXIF before upload',
      updatedAt: ago(4 * DAY),
      status: 'idle',
      projectId: 'p-assets',
      pinned: false,
      unread: false
    },
    // dotfiles-and-machine-setup-scripts
    {
      id: 'c-zsh',
      title: 'zsh prompt shows the worktree branch',
      updatedAt: ago(12 * DAY),
      status: 'idle',
      projectId: 'p-dotfiles',
      pinned: false,
      unread: false
    },
    // The other projects, one conversation each, so the list shows them.
    {
      id: 'c-mini-build',
      title: 'Build the release on the mini',
      updatedAt: ago(5 * DAY),
      status: 'idle',
      projectId: 'p-demi-mini',
      pinned: false,
      unread: false
    },
    {
      id: 'c-api-pages',
      title: 'Paginate the orders endpoint',
      updatedAt: ago(6 * DAY),
      status: 'idle',
      projectId: 'p-api-work',
      pinned: false,
      unread: false
    },
    {
      id: 'c-api-port',
      title: 'Port the old auth middleware',
      updatedAt: ago(20 * DAY),
      status: 'idle',
      projectId: 'p-api-old',
      pinned: false,
      unread: false
    },
    {
      id: 'c-album',
      title: 'Group the photos by trip',
      updatedAt: ago(7 * DAY),
      status: 'idle',
      projectId: 'p-photo-album',
      pinned: false,
      unread: false
    },
    {
      id: 'c-build-ssh',
      title: 'Copy the SSH config to the build box',
      updatedAt: ago(14 * DAY),
      status: 'idle',
      projectId: 'p-dotfiles-build',
      pinned: false,
      unread: false
    },
    {
      id: 'c-old-parser',
      title: 'Try a hand-written parser',
      updatedAt: ago(30 * DAY),
      status: 'idle',
      projectId: 'p-old',
      pinned: false,
      unread: false
    },
    {
      id: 'c-notebook',
      title: 'Plot last month’s signups',
      updatedAt: ago(8 * DAY),
      status: 'idle',
      projectId: 'p-notebooks',
      pinned: false,
      unread: false
    },
  ]
}

/**
 * `projects` after Saved Messages' switch on the project `id`, as the product's leaves the project:
 * to per project, the conversations' lists join its own, which the gallery keeps no count of; to
 * per conversation, its list goes to the conversation `to`. A toast says what moved.
 */
export function savedScopeSwitched(
  projects: readonly SidebarProject[],
  id: string,
  scope: SavedScope,
  to: string | null,
): SidebarProject[] {
  productWould(scope === 'project'
    ? 'Merge the Conversations’ Saved Messages into the Project'
    : to ? 'Move the Project’s Saved Messages to the Conversation' : 'Keep Saved Messages per Conversation')
  return projects.map((project) => (project.id === id ? { ...project, savedScope: scope, savedCount: 0 } : project))
}
