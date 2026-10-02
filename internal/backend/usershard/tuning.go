package usershard

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import "time"

// LifecycleTuning configures idle release and retention.
type LifecycleTuning struct {
	// How long a conversation, or every conversation using the Cloud, stays
	// inactive before its Host's resources are reclaimed.
	IdleWindow time.Duration
	// How often a conversation's idle watch reads its activity.
	IdlePoll time.Duration
	// How long after one retention pass of every user the next starts; the
	// first starts once the backend serves. None runs no pass by itself,
	// for tests that run a user's pass when they choose
	// (`Backend::run_retention`).
	RetentionInterval *time.Duration
}

// DefaultLifecycleTuning returns the production timing and bounds.
func DefaultLifecycleTuning() LifecycleTuning { panic("not written: b-usershard") }

// ExposeTuning configures relay connection idle time.
type ExposeTuning struct {
	// A relayed connection on which no byte moved for this long is closed.
	Idle time.Duration
}

// DefaultExposeTuning returns the production timing and bounds.
func DefaultExposeTuning() ExposeTuning { panic("not written: b-usershard") }

// ConversationTuning configures conversation delivery and rate limits.
type ConversationTuning struct {
	// The most frames a conversation socket's outbox holds; a client that
	// falls this far behind is disconnected as lagging.
	OutboxFrames int
	// The provider requests a user's conversations may start in any minute.
	RequestsPerMinute int
	// Whether the backend asks the conversation's model for a title
	// (`product.md` § Conversation titles); off, a title stays the one the
	// first message gives. A test whose scripted vendor answers only the
	// turns turns it off.
	Titles bool
}

// DefaultConversationTuning returns the production timing and bounds.
func DefaultConversationTuning() ConversationTuning { panic("not written: b-usershard") }

// PageTuning configures page socket timing.
type PageTuning struct {
	// A socket that has sent nothing for this long sends a heartbeat, so
	// that its page can tell it from a dead one.
	Heartbeat time.Duration
	// How long a socket's close frame waits for a page that does not read;
	// a page that has not taken it by then loses the connection without it
	// (`backend.md` § Startup and shutdown).
	CloseWait time.Duration
}

// DefaultPageTuning returns the production timing and bounds.
func DefaultPageTuning() PageTuning { panic("not written: b-usershard") }

// RunnerTuning configures runner handshakes and liveness.
type RunnerTuning struct {
	// A connection that sends no hello within this is closed.
	HelloDeadline time.Duration
	// How long a pairing code lives before its waiting runner gets a new one.
	ClaimLifetime time.Duration
	// How many pairing codes one user may try within a minute.
	ClaimsPerMinute int
	// How often a connected runner is asked whether it is there; none turns
	// liveness off.
	Ping *time.Duration
}

// DefaultRunnerTuning returns the production timing and bounds.
func DefaultRunnerTuning() RunnerTuning { panic("not written: b-usershard") }
