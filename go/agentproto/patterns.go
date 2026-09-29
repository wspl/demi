package agentproto

import "regexp"

var (
	ClientContentFileNamePattern = regexp.MustCompile(`^(?:[^./\\\x00][^/\\\x00]*|\.[^./\\\x00][^/\\\x00]*|\.\.[^/\\\x00]+)$`)
	ClientContentPathPattern     = regexp.MustCompile(`^/[^\x00]*$`)
)
