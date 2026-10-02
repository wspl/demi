package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

import (
	"context"
	"os"
)

// LiveInputEnv identifies the inherited live input reference.
const LiveInputEnv = "DEMI_LIVE_INPUT"

// StandardFile duplicates stdin for 0, stdout for 1, and stderr otherwise.
// The caller owns and closes the returned file.
func StandardFile(ctx context.Context, descriptor uint32) (*os.File, error) {
	panic("not written: r-process")
}

// LiveReference identifies a live input file. The caller keeps the file open
// for the entire shell job.
func LiveReference(file *os.File) (string, error) { panic("not written: r-process") }

// IsLive compares file with the live input reference in env.
func IsLive(file *os.File, env map[string]string) (bool, error) { panic("not written: r-process") }
