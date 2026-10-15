import { GALLERY_MARKS } from '../fixtures/device-marks'
import type { SidebarAccount, SidebarConversation, SidebarProject } from '@demicodes/web-ui/sidebar/types'
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
      path: '/Users/alex/Projects/shop'
    },
    {
      id: 'p-demi-mini',
      name: 'demi',
      hostKind: 'device',
      device: { kind: 'paired', name: 'mac-mini', state: 'online', mark: GALLERY_MARKS.macMini },
      path: '/Users/alex/Projects/shop'
    },
    {
      id: 'p-shop',
      // The only project of its name: the mark alone shows its device, whose name shows in its tip.
      name: 'photo-album',
      hostKind: 'device',
      device: { kind: 'paired', name: 'Alexs-MacBook-Pro.local', state: 'online', mark: GALLERY_MARKS.macPro },
      path: '/Users/alex/Projects/photo-album'
    },
    {
      id: 'p-assets',
      // A long name, cut at its end; the device is offline.
      name: 'renderfarm-render-pipeline',
      hostKind: 'device',
      device: { kind: 'paired', name: 'build-01', state: 'offline', mark: GALLERY_MARKS.ubuntu },
      path: '/srv/renderfarm-render-pipeline'
    },
    {
      id: 'p-dotfiles',
      // A long name on a long device's name, with a namesake on another device: the device's
      // name is cut at half of the text, the project's name in the rest; each shows whole on hover.
      name: 'dotfiles-and-machine-setup-scripts',
      hostKind: 'device',
      // Its runner replaces itself with the server's release.
      device: { kind: 'paired', name: 'Alexs-MacBook-Pro.local', state: 'updating', mark: GALLERY_MARKS.macPro },
      path: '/Users/alex/dotfiles-and-machine-setup-scripts'
    },
    {
      id: 'p-dotfiles-build',
      name: 'dotfiles-and-machine-setup-scripts',
      hostKind: 'device',
      device: { kind: 'paired', name: 'build-01', state: 'offline', mark: GALLERY_MARKS.ubuntu },
      path: '/home/build/dotfiles-and-machine-setup-scripts'
    },
    {
      id: 'p-old',
      // Its device was removed since: the mark has no dot.
      name: 'old-experiments',
      hostKind: 'device',
      device: { kind: 'removed' },
      path: '/Users/alex/old-experiments'
    },
    {
      id: 'p-notebooks',
      name: 'notebooks',
      hostKind: 'cloud',
      path: '/home/demi/notebooks'
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
      title: 'Release note wording for the tier contract',
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
    },
    {
      id: 'c-cookie',
      title: 'Cookie helper refactor',
      updatedAt: ago(40 * 60 * 1000),
      status: 'done',
      projectId: 'p-demi',
      pinned: false,
      unread: true
    },
    {
      id: 'c-gallery',
      title: 'Gallery overlay wells',
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
      title: 'Session tabs drag and drop',
      updatedAt: ago(DAY + 2 * HOUR),
      status: 'idle',
      projectId: 'p-demi',
      pinned: false,
      unread: false
    },
    {
      id: 'c-shell',
      title: 'Shell preview budget as a function of the context window',
      updatedAt: ago(3 * DAY),
      status: 'idle',
      projectId: 'p-demi',
      pinned: false,
      unread: false
    },
    // renderfarm
    {
      id: 'c-thumbs',
      title: 'Thumbnail pipeline on the new bucket',
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
  ]
}
