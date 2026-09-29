package controlproto

// A Tuning is the tuning file: it sets what no environment variable sets, in a
// test build only (the tuning structs of the backend's configuration, the
// models.dev address and the user streams). A member left out keeps the
// product's default. Durations are milliseconds.
//
//demi:wire
type Tuning struct {
	Runners       *RunnersTuning       `json:"runners,omitzero"`
	Lifecycle     *LifecycleTuning     `json:"lifecycle,omitzero"`
	Cloud         *CloudTuning         `json:"cloud,omitzero"`
	Conversations *ConversationsTuning `json:"conversations,omitzero"`
	Pages         *PagesTuning         `json:"pages,omitzero"`
	Exposes       *ExposesTuning       `json:"exposes,omitzero"`
	Logins        *LoginsTuning        `json:"logins,omitzero"`
	// ModelsDevURL is where the backend reads the models.dev document.
	ModelsDevURL *string `json:"modelsDevUrl,omitzero" check:"chars=1.."`
	// UserStreams are the user streams a page may open, by name, each bound to
	// an operation of a native package; they replace the default one.
	UserStreams *map[string]NativeOperation `json:"userStreams,omitzero"`
	// Families points the subscription families at the suite's scripted
	// servers in place of their vendors.
	Families *FamiliesTuning `json:"families,omitzero"`
	// ClockStartMs is where the manual clock starts, in Unix milliseconds; the
	// suite's shared start without it.
	ClockStartMs *int64 `json:"clockStartMs,omitzero"`
	// Mail is whether the backend has a mail sender: one that keeps what it
	// sends for mail.list. Without one an email change answers
	// mail_unavailable.
	Mail *bool `json:"mail,omitzero"`
}

// FamiliesTuning is where the subscription families reach their vendors; a
// family left out keeps its vendor's endpoints.
//
//demi:wire
type FamiliesTuning struct {
	Codex *CodexTuning `json:"codex,omitzero"`
}

// CodexTuning is the ChatGPT backend and the sign-in service of the codex
// family.
//
//demi:wire
type CodexTuning struct {
	BackendURL string `json:"backendUrl" check:"chars=1.."`
	AuthURL    string `json:"authUrl" check:"chars=1.."`
}

// A NativeOperation is the operation of a native package that a user stream
// runs.
//
//demi:wire
type NativeOperation struct {
	Package   string `json:"package" check:"chars=1.."`
	Operation string `json:"operation" check:"chars=1.."`
}

// RunnersTuning is how the backend treats runner connections.
//
//demi:wire
type RunnersTuning struct {
	// HelloDeadlineMs closes a connection that sends no hello within it.
	HelloDeadlineMs *uint64 `json:"helloDeadlineMs,omitzero"`
	// ClaimLifetimeMs is how long a pairing code lives.
	ClaimLifetimeMs *uint64 `json:"claimLifetimeMs,omitzero"`
	// ClaimsPerMinute is how many pairing codes one user may try in a minute.
	ClaimsPerMinute *uint64 `json:"claimsPerMinute,omitzero"`
	// PingMs is how often a connected runner is asked whether it is there; 0
	// turns liveness off.
	PingMs *uint64 `json:"pingMs,omitzero"`
}

// LifecycleTuning is when a conversation's Host resources are reclaimed and
// how often the retention pass runs.
//
//demi:wire
type LifecycleTuning struct {
	IdleWindowMs *uint64 `json:"idleWindowMs,omitzero"`
	IdlePollMs   *uint64 `json:"idlePollMs,omitzero"`
	// RetentionIntervalMs is how long after one retention pass of every user
	// the next starts; 0 runs no pass by itself.
	RetentionIntervalMs *uint64 `json:"retentionIntervalMs,omitzero"`
}

// CloudTuning is how the backend runs each user's Cloud.
//
//demi:wire
type CloudTuning struct {
	SweepMs              *uint64 `json:"sweepMs,omitzero"`
	CheckpointIntervalMs *uint64 `json:"checkpointIntervalMs,omitzero"`
	LifetimeCapMs        *uint64 `json:"lifetimeCapMs,omitzero"`
	RunnerConnectionMs   *uint64 `json:"runnerConnectionMs,omitzero"`
	CrashLoopDeaths      *uint32 `json:"crashLoopDeaths,omitzero"`
	CrashLoopWindowMs    *uint64 `json:"crashLoopWindowMs,omitzero"`
	SyncTimeoutMs        *uint64 `json:"syncTimeoutMs,omitzero"`
	ResetHoldMs          *uint64 `json:"resetHoldMs,omitzero"`
	SystemQuota          *uint64 `json:"systemQuota,omitzero"`
	HomeQuota            *uint64 `json:"homeQuota,omitzero"`
	Capacity             *uint64 `json:"capacity,omitzero"`
}

// ConversationsTuning is how the backend serves conversations.
//
//demi:wire
type ConversationsTuning struct {
	// OutboxFrames is the most frames a conversation socket's outbox holds.
	OutboxFrames *uint64 `json:"outboxFrames,omitzero"`
	// RequestsPerMinute is the provider requests a user's conversations may
	// start in a minute.
	RequestsPerMinute *uint64 `json:"requestsPerMinute,omitzero"`
	// Titles is whether the backend asks the conversation's model for a title.
	Titles *bool `json:"titles,omitzero"`
}

// PagesTuning is how the backend times its sockets to a page.
//
//demi:wire
type PagesTuning struct {
	HeartbeatMs *uint64 `json:"heartbeatMs,omitzero"`
	CloseWaitMs *uint64 `json:"closeWaitMs,omitzero"`
}

// ExposesTuning is how the public relay treats its connections.
//
//demi:wire
type ExposesTuning struct {
	IdleMs *uint64 `json:"idleMs,omitzero"`
}

// LoginsTuning is how long a device login waits for its user and how long its
// result is kept.
//
//demi:wire
type LoginsTuning struct {
	LifetimeMs  *uint64 `json:"lifetimeMs,omitzero"`
	RetentionMs *uint64 `json:"retentionMs,omitzero"`
}

// EncodeTuning returns the tuning file's JSON.
func EncodeTuning(tuning Tuning) ([]byte, error) {
	return encode(tuning)
}

// DecodeTuning decodes a tuning file.
func DecodeTuning(data []byte) (Tuning, error) {
	return decode[Tuning](data)
}
