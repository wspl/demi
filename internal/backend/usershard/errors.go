package usershard

//revive:disable:unused-parameter

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
	Kind    ForkErrorKind
	Message string
	Err     error
}

// Error returns the refusal in the reference spelling.
func (e *ForkRefusal) Error() string { panic("not written: b-usershard") }

// Unwrap returns the underlying failure, if any.
func (e *ForkRefusal) Unwrap() error { panic("not written: b-usershard") }

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
	Kind    TitleErrorKind
	Message string
	Err     error
}

// Error returns the refusal in the reference spelling.
func (e *TitleRefusal) Error() string { panic("not written: b-usershard") }

// Unwrap returns the underlying failure, if any.
func (e *TitleRefusal) Unwrap() error { panic("not written: b-usershard") }

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
	Kind    ReloadErrorKind
	Message string
	Err     error
}

// Error returns the refusal in the reference spelling.
func (e *ReloadRefusal) Error() string { panic("not written: b-usershard") }

// Unwrap returns the underlying failure, if any.
func (e *ReloadRefusal) Unwrap() error { panic("not written: b-usershard") }

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
	Kind    ServicesErrorKind
	Message string
	Err     error
}

// Error returns the refusal in the reference spelling.
func (e *ServicesError) Error() string { panic("not written: b-usershard") }

// Unwrap returns the underlying failure, if any.
func (e *ServicesError) Unwrap() error { panic("not written: b-usershard") }

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
	Kind    CloseErrorKind
	Message string
	Err     error
}

// Error returns the refusal in the reference spelling.
func (e *CloseError) Error() string { panic("not written: b-usershard") }

// Unwrap returns the underlying failure, if any.
func (e *CloseError) Unwrap() error { panic("not written: b-usershard") }

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
	Kind    UnavailableKind
	Message string
	Err     error
}

// Error returns the refusal in the reference spelling.
func (e *ShardUnavailable) Error() string { panic("not written: b-usershard") }

// Unwrap returns the underlying failure, if any.
func (e *ShardUnavailable) Unwrap() error { panic("not written: b-usershard") }
