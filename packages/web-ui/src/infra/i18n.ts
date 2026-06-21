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
  'common.continue': 'Continue',
  'agent.block.thinking': 'Thinking',
  'agent.block.error': 'Error',
  'agent.block.stackTrace': 'Stack trace',
  'agent.block.aborted': 'Aborted',
  'agent.block.responseStats': 'Response',
  'agent.stats.contextWindow': 'Context window',
  'agent.stats.cacheRead': 'Cache read',
  'agent.stats.input': 'Input',
  'agent.stats.output': 'Output',
  'agent.stats.cacheWrite': 'Cache write',
  'agent.stats.totalUsage': 'Total usage',
  'agent.stats.unavailable': 'N/A',
  'providers.model.thinking': 'Thinking',
  'providers.reasoning': 'Reasoning effort',
  'agent.context.compacting': 'Compacting context…',
  'agent.context.noUsage': 'No usage recorded yet',
  'agent.context.unavailable': 'Context usage unavailable',
  'agent.context.compactHint': 'Click to compact context',
}

export function t(key: string): string {
  return messages[key] ?? key
}
