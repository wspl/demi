//go:build !linux

package procgroup

import (
	"fmt"
	"runtime"
)

// AdoptOrphans refuses platforms without this harness's Linux subreaper and
// /proc ownership checks; group cleanup alone cannot find detached orphans.
func AdoptOrphans() error {
	return fmt.Errorf("backendtest/procgroup: orphan adoption is unsupported on %s; run the suite on Linux", runtime.GOOS)
}

// KillAdopted has nothing to reap because AdoptOrphans cannot succeed here.
func KillAdopted(root string) {}
