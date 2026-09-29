package hostremotetest

import "os"

func terminate(process *os.Process) error { return process.Kill() }
