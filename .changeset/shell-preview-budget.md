---
'@demicodes/agent': minor
---

Shell tool-result preview budgets are ten times larger (10k tokens below an 800k context window, 100k at and above), and `AgentServerOptions.tools.shellPreviewBudgetTokens` pins a fixed budget for the root session and every subagent instead of the context-window heuristic.
