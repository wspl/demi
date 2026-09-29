package shell

import (
	"context"
	"errors"
)

var resourceNumbers = map[rune]int{}

const pipeBuffer = 512

func infinity() uint64 { return ^uint64(0) }
func resourceLimit(context.Context, int) (uint64, uint64, error) {
	return 0, 0, errors.New("resource limits are not supported on Windows")
}
func executeHelper(childSetup) error {
	return errors.New("child attributes are not supported on Windows")
}
