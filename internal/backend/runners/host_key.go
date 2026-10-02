package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import "github.com/wspl/demi/internal/host"

// ConversationOf returns the conversation a Host key names, if it is a
// conversation's Host. Device and machine access return false.
func ConversationOf(key host.Key) (string, bool) { panic("not written: b-runners") }

// DeviceOf returns the device a conversation's Host key names, its third word.
func DeviceOf(key host.Key) (string, bool) { panic("not written: b-runners") }
