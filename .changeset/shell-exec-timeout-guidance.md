---
"@demicodes/agent": patch
---

Guide shell_exec timeoutMs toward the command's expected run time so results return in the same call instead of through extra shell_status round trips, and ask for independent commands to be issued together in one response.
