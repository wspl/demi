---
'@demicodes/agent': minor
---

Shell tool-result preview budgets are ten times larger (10k tokens below an 800k context window, 100k at and above), and `AgentServerOptions.tools.shellPreviewBudgetTokens` (`(contextWindow) => tokens`) replaces that split for the root session and every subagent.
