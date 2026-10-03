package database

import (
	"errors"
	"fmt"
)

// ErrWorkspaceNotFound means the user has no workspace of this ID.
var ErrWorkspaceNotFound = errors.New("no such workspace")

// WorkspaceInUseError means conversations still target the workspace.
type WorkspaceInUseError struct {
	// Count is how many conversations target it.
	Count uint64
}

// Error says how many conversations still target the workspace.
func (e *WorkspaceInUseError) Error() string {
	return fmt.Sprintf("%d conversation(s) still target this workspace", e.Count)
}
