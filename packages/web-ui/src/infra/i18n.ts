// Minimal English string table. Keys mirror agent-gui's i18n keys for the ported
// tab/list/input surfaces; unmapped keys fall back to the key itself.

const messages: Record<string, string> = {
  'common.rename': 'Rename',
  'common.close': 'Close',
  'agent.noWorkspace': 'No workspace open',
  'agent.newConversation': 'New conversation',
  'agent.tab.newTabRight': 'New tab to the right',
  'agent.tab.closeOthers': 'Close others',
  'agent.tab.closeToLeft': 'Close tabs to the left',
  'agent.tab.closeToRight': 'Close tabs to the right',
  'agent.tab.closeAll': 'Close all tabs',
  'agent.tab.copyConversationId': 'Copy conversation ID',
}

export function t(key: string): string {
  return messages[key] ?? key
}
