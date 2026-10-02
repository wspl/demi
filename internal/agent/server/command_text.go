package server

const spawnPrompt = "The child's first user message and only task brief. The child starts with an empty transcript and cannot see this conversation: do not refer to prior turns, and do not paste this conversation or the product user's message unchanged. Include the goal for this child, applicable decisions and constraints, whether to edit or only report, how to verify, and every concrete identifier it needs (paths, ids, error text, commands already tried and their key results). State the exact shape of the last assistant text it should return."

const spawnSummary = "Start a child agent session and return its id immediately after creation. The child runs independently of this command. Completion arrives as a message to the parent, waking it when idle. When you have no independent work left, end your turn and let the completion message wake you. Do not poll agent list/show or schedule timed yield calls to wait for children. Use agent send to communicate and agent abort to stop it. Children can spawn children of their own."

const sendSummary = "Deliver information to any live agent in the tree, or parent. A busy recipient incorporates it through internal steering; an idle recipient wakes. Returns after durable acceptance, without waiting for an answer. Use for interim information, questions, or blockers; your final answer is delivered automatically. Archived recipients must be reopened by their parent with resume."

const abortSummary = "Abort one of your own running children and its whole subtree. Siblings are untouched; only the spawning session may abort a child."

const resumeSummary = "Revive one of your own archived children with a new user message on its preserved transcript. Return its id immediately after accepting the message; completion is delivered separately to the parent. Use agent send to communicate and agent abort to stop it. Archived ids are in agent list."

const listSummary = "Render the whole session tree from the root down, marking your own position. Live agents show phase, ages, execution, and activity; each node's archived (finished, revivable by its parent) children render beneath it. Every age is relative to now. A read, not a wait — not for polling loops."

const showSummary = "Bounded snapshot of any live agent in the tree (root excluded): execution state, recent tool titles with durations, last assistant text. Every duration is relative to now — use the ages to tell motion from stall. Omits tool outputs, file contents, and older turns. A read, not a wait — not for polling loops."

const shellGroupSummary = "Shell commands: read a command's whole output."
const shellOutputSummary = "Print a command's whole output by its commandId: numbered lines a page at a time, as `cat -n` shows them, from the first line or the lines --lines <from>-<to> names; the newest with --tail <n>. --stdout or --stderr takes one stream, with line numbers of its own. --raw prints the bytes as they are, unnumbered and unpaged, for pipes and files: `grep -n` on it gives the numbers --lines takes (`demi shell output 17 --raw | grep -n FAIL`). Any command of this conversation, running or ended."
