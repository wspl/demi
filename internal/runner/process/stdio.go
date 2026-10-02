package process

import (
	"context"
	"os"
)

// LiveInputEnv identifies the inherited live input reference.
const LiveInputEnv = "DEMI_LIVE_INPUT"

// StandardFile duplicates stdin for 0, stdout for 1, and stderr otherwise.
// The caller owns and closes the returned file.
func StandardFile(ctx context.Context, descriptor uint32) (*os.File, error) {
	return duplicateStandard(ctx, descriptor)
}

// LiveReference identifies a live input file. The caller keeps the file open
// for the entire shell job.
func LiveReference(file *os.File) (string, error) { return inputReference(file) }

// IsLive compares file with the live input reference in env.
func IsLive(file *os.File, env map[string]string) (bool, error) {
	reference, ok := env[LiveInputEnv]
	if !ok {
		return false, nil
	}
	return matchesReference(file, reference)
}
