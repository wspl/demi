package host

import "slices"

// ReservedNames lists shell words, builtins, Unix tools and toolchains that a root cannot shadow.
func ReservedNames() []string {
	return []string{".", "bash", "break", "cd", "command", "continue", "echo", "exit", "export", "jobs", "local", "popd", "printf", "pushd", "read", "return", "set", "sh", "shift", "source", "test", "true", "false", "unset", "wait", "awk", "cat", "chmod", "cp", "cut", "du", "file", "find", "grep", "head", "jq", "ls", "mkdir", "mv", "nl", "rg", "rm", "sed", "sort", "stat", "tail", "tee", "touch", "tr", "tree", "uniq", "wc", "xargs", "yq", "bun", "cargo", "docker", "git", "go", "node", "npm", "pnpm", "python", "python3", "ruby", "rustc", "yarn"}
}

// IsReserved reports whether a root name belongs to the shell or system.
func IsReserved(name string) bool { return slices.Contains(ReservedNames(), name) }
