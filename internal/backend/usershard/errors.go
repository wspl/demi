package usershard

import "fmt"

// ForkErrorKind identifies why a Fork was refused.
type ForkErrorKind uint8

const (
	// ForkSourceNotFound means: No such conversation
	ForkSourceNotFound ForkErrorKind = iota
	// ForkConflict means: The Fork’s id belongs to another creation attempt
	ForkConflict
	// ForkUnavailable means: Conversation id is unavailable
	ForkUnavailable
	// ForkTarget means: The text is not a completed assistant text.
	ForkTarget
	// ForkStorage means: Storage failed.
	ForkStorage
	// ForkFailed means: The agent could not store the destination root.
	ForkFailed
)

// ForkRefusal reports why a Fork was refused. Its cause is available through errors.As.
type ForkRefusal struct {
	// Kind selects the failure category and applicable details.
	Kind ForkErrorKind
	// Message holds the diagnostic for categories with custom text.
	Message string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
}

// Error returns the refusal in the reference spelling.
func (e *ForkRefusal) Error() string {
	switch e.Kind {
	case ForkSourceNotFound:
		return "No such conversation"
	case ForkConflict:
		return "The Fork's id belongs to another creation attempt"
	case ForkUnavailable:
		return "Conversation id is unavailable"
	case ForkTarget:
		return e.Message
	case ForkStorage:
		return fmt.Sprintf("%v", e.Err)
	case ForkFailed:
		return e.Message
	}
	return e.Message
}

// Unwrap returns the underlying failure, if any.
func (e *ForkRefusal) Unwrap() error { return e.Err }

// TitleErrorKind identifies why a title was refused.
type TitleErrorKind uint8

const (
	// TitleNotFound means: No such conversation
	TitleNotFound TitleErrorKind = iota
	// TitleArchived means: Restore the conversation before changing it
	TitleArchived
	// TitleProviderNotFound means: No such provider
	TitleProviderNotFound
	// TitleModelNotSelected means: Choose a model for the conversation first
	TitleModelNotSelected
	// TitleNoMessages means: The conversation has no message to title
	TitleNoMessages
	// TitleStorage means: Storage failed.
	TitleStorage
)

// TitleRefusal reports why a title was refused. Its cause is available through errors.As.
type TitleRefusal struct {
	// Kind selects the failure category and applicable details.
	Kind TitleErrorKind
	// Message holds the diagnostic for categories with custom text.
	Message string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
}

// Error returns the refusal in the reference spelling.
func (e *TitleRefusal) Error() string {
	switch e.Kind {
	case TitleNotFound:
		return "No such conversation"
	case TitleArchived:
		return "Restore the conversation before changing it"
	case TitleProviderNotFound:
		return "No such provider"
	case TitleModelNotSelected:
		return "Choose a model for the conversation first"
	case TitleNoMessages:
		return "The conversation has no message to title"
	case TitleStorage:
		return fmt.Sprintf("%v", e.Err)
	}
	return e.Message
}

// Unwrap returns the underlying failure, if any.
func (e *TitleRefusal) Unwrap() error { return e.Err }

// ReloadErrorKind identifies why a tree was not reloaded.
type ReloadErrorKind uint8

const (
	// ReloadAccess means: Host access refused the request.
	ReloadAccess ReloadErrorKind = iota
	// ReloadArchived means: The conversation is archived
	ReloadArchived
	// ReloadWorking means: The conversation’s agents are working; reload once they are done
	ReloadWorking
)

// ReloadRefusal reports why a tree was not reloaded. Its cause is available through errors.As.
type ReloadRefusal struct {
	// Kind selects the failure category and applicable details.
	Kind ReloadErrorKind
	// Message holds the diagnostic for categories with custom text.
	Message string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
}

// Error returns the refusal in the reference spelling.
func (e *ReloadRefusal) Error() string {
	switch e.Kind {
	case ReloadAccess:
		return fmt.Sprintf("%v", e.Err)
	case ReloadArchived:
		return "The conversation is archived"
	case ReloadWorking:
		return "The conversation's agents are working; reload once they are done"
	}
	return e.Message
}

// Unwrap returns the underlying failure, if any.
func (e *ReloadRefusal) Unwrap() error { return e.Err }

// ServicesErrorKind identifies why shared services could not start.
type ServicesErrorKind uint8

const (
	// ServicesHashing means: Password hashing could not start.
	ServicesHashing ServicesErrorKind = iota
	// ServicesHTTP means: The HTTP client could not start.
	ServicesHTTP
	// ServicesPlugins means: Plugins could not start.
	ServicesPlugins
)

// ServicesError reports why shared services could not start. Its cause is available through errors.As.
type ServicesError struct {
	// Kind selects the failure category and applicable details.
	Kind ServicesErrorKind
	// Message holds the diagnostic for categories with custom text.
	Message string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
}

// Error returns the refusal in the reference spelling.
func (e *ServicesError) Error() string {
	switch e.Kind {
	case ServicesHashing:
		return fmt.Sprintf("password hashing cannot start: %v", e.Err)
	case ServicesHTTP:
		return fmt.Sprintf("the HTTP client cannot start: %v", e.Err)
	case ServicesPlugins:
		return fmt.Sprintf("the plugins cannot start: %v", e.Err)
	}
	return e.Message
}

// Unwrap returns the underlying failure, if any.
func (e *ServicesError) Unwrap() error { return e.Err }

// CloseErrorKind identifies which database did not close.
type CloseErrorKind uint8

const (
	// CloseConversation means: A conversation database did not close.
	CloseConversation CloseErrorKind = iota
	// CloseControl means: The control database did not close.
	CloseControl
)

// CloseError reports which database did not close. Its cause is available through errors.As.
type CloseError struct {
	// Kind selects the failure category and applicable details.
	Kind CloseErrorKind
	// Message holds the diagnostic for categories with custom text.
	Message string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
}

// Error returns the refusal in the reference spelling.
func (e *CloseError) Error() string {
	switch e.Kind {
	case CloseConversation:
		return fmt.Sprintf("a conversation database did not close: %v", e.Err)
	case CloseControl:
		return fmt.Sprintf("the control database did not close: %v", e.Err)
	}
	return e.Message
}

// Unwrap returns the underlying failure, if any.
func (e *CloseError) Unwrap() error { return e.Err }

// UnavailableKind identifies why shard admission failed.
type UnavailableKind uint8

const (
	// ShardClosing means: The backend is shutting down.
	ShardClosing UnavailableKind = iota
	// ShardFailed means: The shard operation failed after a panic.
	ShardFailed
)

// ShardUnavailable reports why shard admission failed. Its cause is available through errors.As.
type ShardUnavailable struct {
	// Kind selects the failure category and applicable details.
	Kind UnavailableKind
	// Message holds the diagnostic for categories with custom text.
	Message string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
}

// Error returns the refusal in the reference spelling.
func (e *ShardUnavailable) Error() string {
	switch e.Kind {
	case ShardClosing:
		return "the backend is shutting down"
	case ShardFailed:
		return "the shard call failed"
	}
	return e.Message
}

// Unwrap returns the underlying failure, if any.
func (e *ShardUnavailable) Unwrap() error { return e.Err }
