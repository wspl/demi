package cmdsdktest

import "github.com/wspl/demi/internal/cmdsdk"

// Pauses is the process-wide count of descriptor retry pauses.
func Pauses() uint64 { return cmdsdk.DescriptorPauses() }
