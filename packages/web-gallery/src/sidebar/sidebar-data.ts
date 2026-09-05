import type { SidebarAccount, SidebarConversation, SidebarExtension, SidebarProject } from '@demicodes/web-ui/sidebar/types'

const HOUR = 60 * 60 * 1000
const DAY = 24 * HOUR

function ago(ms: number): string {
  return new Date(Date.now() - ms).toISOString()
}

export const demoAccount: SidebarAccount = {
  name: 'Zan',
  email: 'zan@example.com',
  plan: 'Pro',
}

export function demoProjects(): SidebarProject[] {
  return [
    { id: 'p-demi', name: 'demi', host: 'zan-mbp', hostKind: 'device', path: '/Users/zan/Projects/demi' },
    { id: 'p-assets', name: 'assetsfactory', host: 'build-01', hostKind: 'device', path: '/srv/assetsfactory' },
    { id: 'p-dotfiles', name: 'dotfiles', host: 'zan-mbp', hostKind: 'device', path: '/Users/zan/dotfiles' },
  ]
}

export function demoConversations(): SidebarConversation[] {
  return [
    // Plain conversations: questions with no checkout behind them.
    { id: 'c-shift', title: 'How does floating-ui limitShift differ from shift?', updatedAt: ago(25 * 60 * 1000), status: 'done', projectId: null, pinned: false, unread: true },
    { id: 'c-wording', title: 'Release note wording for the tier contract', updatedAt: ago(5 * HOUR), status: 'idle', projectId: null, pinned: false, unread: false },
    { id: 'c-regex', title: 'Regex for fenced code lines', updatedAt: ago(2 * DAY), status: 'idle', projectId: null, pinned: false, unread: false },
    // demi
    { id: 'c-login', title: 'Login test after the session cookie rename', updatedAt: ago(4 * 60 * 1000), status: 'active', projectId: 'p-demi', pinned: true, unread: false },
    { id: 'c-cookie', title: 'Cookie helper refactor', updatedAt: ago(40 * 60 * 1000), status: 'done', projectId: 'p-demi', pinned: false, unread: true },
    { id: 'c-gallery', title: 'Gallery overlay wells', updatedAt: ago(3 * HOUR), status: 'idle', projectId: 'p-demi', pinned: false, unread: false },
    { id: 'c-rate', title: 'Rate limit retry policy', updatedAt: ago(6 * HOUR), status: 'error', projectId: 'p-demi', pinned: false, unread: true },
    { id: 'c-tabs', title: 'Session tabs drag and drop', updatedAt: ago(DAY + 2 * HOUR), status: 'idle', projectId: 'p-demi', pinned: false, unread: false },
    { id: 'c-shell', title: 'Shell preview budget as a function of the context window', updatedAt: ago(3 * DAY), status: 'idle', projectId: 'p-demi', pinned: false, unread: false },
    // assetsfactory
    { id: 'c-thumbs', title: 'Thumbnail pipeline on the new bucket', updatedAt: ago(DAY + 9 * HOUR), status: 'aborted', projectId: 'p-assets', pinned: false, unread: true },
    { id: 'c-exif', title: 'Strip EXIF before upload', updatedAt: ago(4 * DAY), status: 'idle', projectId: 'p-assets', pinned: false, unread: false },
    // dotfiles
    { id: 'c-zsh', title: 'zsh prompt shows the worktree branch', updatedAt: ago(12 * DAY), status: 'idle', projectId: 'p-dotfiles', pinned: false, unread: false },
  ]
}

export function demoPlugins(): SidebarExtension[] {
  return [
    { id: 'github', name: 'GitHub', summary: 'Pull requests, issues, and checks', enabled: true },
    { id: 'browser', name: 'Browser', summary: 'Drive a page and read what it renders', enabled: true },
    { id: 'database', name: 'Database', summary: 'Query the development database', enabled: false },
    { id: 'notify', name: 'Notifications', summary: 'Ping when a long turn finishes', enabled: true },
  ]
}

export function demoSkills(): SidebarExtension[] {
  return [
    { id: 'commit', name: 'commit', summary: 'Conventional commit from the staged diff', enabled: true },
    { id: 'review', name: 'review', summary: 'Adversarial review of the current branch', enabled: true },
    { id: 'release', name: 'release', summary: 'Version packages and write the changelog', enabled: true },
    { id: 'e2e', name: 'e2e', summary: 'Run the browser suite against the dev server', enabled: false },
    { id: 'docs', name: 'docs', summary: 'Keep the design records in step with the code', enabled: false },
  ]
}
