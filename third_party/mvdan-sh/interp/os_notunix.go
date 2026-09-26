// Copyright (c) 2017, Andrey Nering <andrey.nering@gmail.com>
// See LICENSE for licensing information

//go:build !unix

package interp

import (
	"context"
	"fmt"
	"os"
	"strings"

	"mvdan.cc/sh/v3/syntax"
)

// sigPipe is the number that Unix gives the SIGPIPE signal.
const sigPipe = 13

// signalNumber knows the numbers that Unix gives the signals common to all
// Unix systems, as there are no signals to send here.
func signalNumber(name string) (int, bool) {
	switch strings.TrimPrefix(name, "SIG") {
	case "HUP":
		return 1, true
	case "INT":
		return 2, true
	case "QUIT":
		return 3, true
	case "KILL":
		return 9, true
	case "TERM":
		return 15, true
	}
	return 0, false
}

// fileDescriptor is unsupported, as process substitutions are.
func fileDescriptor(f *os.File) (int, error) {
	return 0, fmt.Errorf("unsupported")
}

// defaultAccess attempts to emulate access(2) on Windows.
// Windows seems to have a different system of permissions than Unix,
// so for now just rely on what [io/fs.FileInfo] gives us
// via the stat handler.
func defaultAccess(ctx context.Context, path string, mode AccessMode) error {
	info, err := HandlerCtx(ctx).runner.statHandler(ctx, path, false)
	if err != nil {
		return err
	}
	m := info.Mode()
	if mode&AccessRead != 0 && m&0o400 == 0 {
		return fmt.Errorf("file is not readable")
	}
	if mode&AccessWrite != 0 && m&0o200 == 0 {
		return fmt.Errorf("file is not writable")
	}
	if mode&AccessExec != 0 && m&0o100 == 0 {
		return fmt.Errorf("file is not executable")
	}
	return nil
}

// unTestOwnOrGrp panics. Under Unix, it implements the -O and -G unary tests,
// but under Windows, it's unclear how to implement those tests, since Windows
// doesn't have the concept of a file owner, just ACLs, and it's unclear how
// to map the one to the other.
func (r *Runner) unTestOwnOrGrp(ctx context.Context, op syntax.UnTestOperator, x string) bool {
	r.errf("unsupported unary test op: %v\n", op)
	return false
}

// waitStatus is a no-op on plan9 and windows.
type waitStatus struct{}

// isENOEXEC is a no-op on plan9 and windows.
func isENOEXEC(err error) bool { return false }

// isETXTBSY is a no-op on plan9 and windows.
func isETXTBSY(err error) bool { return false }

func (waitStatus) Signaled() bool { return false }
func (waitStatus) Signal() int    { return 0 }
