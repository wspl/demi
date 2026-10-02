package runner

import "os"

func startupLimits()               {}
func terminationSignal() os.Signal { return os.Interrupt }
