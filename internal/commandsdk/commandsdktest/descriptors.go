package commandsdktest

import "github.com/wspl/demi/internal/commandsdk"

// Pauses is the process-wide count of descriptor retry pauses.
func Pauses() uint64 {
	return commandsdk.DescriptorPauses()
}
