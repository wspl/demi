package commandpackages

import "github.com/wspl/demi/internal/runner/process"

// serviceExitStatus renders a recorded OS status, or the owner's wait failure
// when no OS status exists. An empty record has neither status nor diagnostic.
func serviceExitStatus(exit process.Exit, waitErr error) string {
	if exit.Code != nil || exit.Signal != nil {
		return platformExitStatus(exit)
	}
	if waitErr != nil {
		return waitErr.Error()
	}
	return "unknown exit status"
}
