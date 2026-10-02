package runners

import (
	"strings"

	"github.com/wspl/demi/internal/host"
)

// ConversationOf returns the conversation a Host key names, if it is a
// conversation's Host. Device and machine access return false.
func ConversationOf(key host.Key) (string, bool) {
	words := strings.SplitN(string(key), " ", 3)
	if len(words) < 2 || words[0] != "conversation" {
		return "", false
	}
	return words[1], true
}

// DeviceOf returns the device a conversation's Host key names, its third word.
func DeviceOf(key host.Key) (string, bool) {
	words := strings.SplitN(string(key), " ", 4)
	if len(words) < 3 || words[0] != "conversation" {
		return "", false
	}
	return words[2], true
}
