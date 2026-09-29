package webapi

import "regexp"

var (
	ConversationTargetPathPattern = regexp.MustCompile("^/")
	EmailChangeConfirmCodePattern = regexp.MustCompile("^[0-9]{6}$")
)
