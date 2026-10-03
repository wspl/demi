package usershard

import "errors"

// Refusals of conversation operations the edge answers with their own codes.
//
//nolint:staticcheck // Product text, shown to the user as it is.
var (
	// ErrConversationNotFound means the user has no such conversation.
	ErrConversationNotFound = errors.New("No such conversation")
	// ErrForkConflict means the Fork's id belongs to another creation attempt.
	ErrForkConflict = errors.New("The Fork's id belongs to another creation attempt")
	// ErrIDUnavailable means the Fork's destination id is taken.
	ErrIDUnavailable = errors.New("Conversation id is unavailable")
	// ErrArchivedChange means an archived conversation cannot be changed.
	ErrArchivedChange = errors.New("Restore the conversation before changing it")
	// ErrArchived means an archived conversation cannot be reloaded.
	ErrArchived = errors.New("The conversation is archived")
	// ErrProviderNotFound means the conversation's provider does not exist.
	ErrProviderNotFound = errors.New("No such provider")
	// ErrModelNotSelected means the conversation has no model yet.
	ErrModelNotSelected = errors.New("Choose a model for the conversation first")
	// ErrNoMessages means the conversation has no message to title.
	ErrNoMessages = errors.New("The conversation has no message to title")
	// ErrAgentsWorking means the conversation's agents are working, so it cannot reload.
	ErrAgentsWorking = errors.New("The conversation's agents are working; reload once they are done")
)

// ErrClosing means the backend is shutting down and admits no new shard call.
var ErrClosing = errors.New("the backend is shutting down")

// errShardFailed means a shard call panicked; the shard stays usable.
var errShardFailed = errors.New("the shard call failed")

// ForkTargetError means a Fork cannot start at its target. Its text is the
// cause's; the edge answers it with invalid_fork_target.
type ForkTargetError struct {
	// Err is why the target cannot start a Fork.
	Err error
}

// Error returns the cause's text.
func (e *ForkTargetError) Error() string { return e.Err.Error() }

// Unwrap returns the cause.
func (e *ForkTargetError) Unwrap() error { return e.Err }
