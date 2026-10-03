package server

import (
	"errors"
	"fmt"
)

// ErrWorking means the conversation's tree works, which a reload does not interrupt.
var ErrWorking = errors.New("the conversation's agents are working")

// ErrProviderUnavailable means no provider entry has the requested id. It
// follows the provider's quoted id, as in: Provider "x" is not available.
var ErrProviderUnavailable = errors.New("is not available")

// ProviderUnavailable returns the error for a provider id with no entry.
func ProviderUnavailable(id string) error {
	//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
	return fmt.Errorf("Provider %q %w", id, ErrProviderUnavailable)
}

//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
var (
	// errNoSession refuses a steer on a connection with no open session.
	errNoSession = errors.New("No session is open on this connection")
	// errQueuedMessageNotFound refuses a steer of a message that is not queued.
	errQueuedMessageNotFound = errors.New("Queued message not found")
)
