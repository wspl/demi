package usershard

//revive:disable:unused-parameter

import (
	"context"
	"net/http"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/webapi"
)

// Shard owns one user's runtime state. One mutex protects short decisions;
// no IO, callback, channel operation or join runs while it is held. Methods
// accept concurrent calls; returned component handles synchronize themselves.
type Shard struct{}

// Shards routes each user to its unique in-process shard and owns every shard's
// workers and sockets. Close refuses new work and joins work already admitted.
type Shards struct{}

// NewShards creates routing over services. ctx is the backend lifetime, not a
// request lifetime. The owner must Close before disposing services.
func NewShards(ctx context.Context, services *Services) (*Shards, error) {
	panic("not written: b-usershard")
}

// Of returns the user's stable shard, creating it on first use. Routing refuses
// new requests once shutdown starts; callers invoke shard methods directly.
func (s *Shards) Of(ctx context.Context, user webapi.UserID) (*Shard, error) {
	panic("not written: b-usershard")
}

// OfWhileClosing returns the user's shard for runner pipe requests needed by
// shutdown. It remains available during draining and refuses after routing has
// fully closed. Ordinary product requests use Of.
func (s *Shards) OfWhileClosing(ctx context.Context, user webapi.UserID) (*Shard, error) {
	panic("not written: b-usershard")
}

// Close drains admitted transitions and sockets and joins all shard workers.
func (s *Shards) Close(ctx context.Context) error { panic("not written: b-usershard") }

// Services returns the immutable shared service handles.
func (s *Shard) Services() *Services { panic("not written: b-usershard") }

// HTTP returns the shard's provider HTTP client.
func (s *Shard) HTTP() *http.Client { panic("not written: b-usershard") }

// Plugins returns the synchronized plugin host of this user.
func (s *Shard) Plugins() *plugins.User { panic("not written: b-usershard") }

// Closed is closed when shutdown begins; it does not signal that draining ended.
func (s *Shard) Closed() <-chan struct{} { panic("not written: b-usershard") }

// HostShard supplies the conversation Host access boundary.
func (s *Shard) HostShard() hostaccess.HostShard { panic("not written: b-usershard") }

// ExposeShard supplies the expose boundary through a private adapter because
// its Control, PublicURL and Exposes signatures differ from other consumers.
func (s *Shard) ExposeShard() expose.ExposeShard { panic("not written: b-usershard") }

// RouteDeaths routes manager death events to the owning user's Cloud until
// deaths closes or ctx ends. Its caller owns and joins the call.
func RouteDeaths(ctx context.Context, deaths <-chan webapi.DeviceID, services *Services, shards *Shards) error {
	panic("not written: b-usershard")
}

// ScheduleRetention runs the first retention pass immediately and subsequent
// passes at the configured interval. Its caller owns and joins the call.
func ScheduleRetention(ctx context.Context, services *Services, shards *Shards) error {
	panic("not written: b-usershard")
}

// RetentionPass collects this user's retained conversation and blob data.
func (s *Shard) RetentionPass(ctx context.Context) error { panic("not written: b-usershard") }

// RecoverForks publishes reserved destinations whose roots committed; other
// destinations remain hidden for retry. Call before serving requests.
func RecoverForks(ctx context.Context, control *database.ControlService, conversations *database.ConversationStores) error {
	panic("not written: b-usershard")
}

// RearmWakeups reopens roots with pending persisted wakeups after startup.
func RearmWakeups(ctx context.Context, control *database.ControlService, shards *Shards) error {
	panic("not written: b-usershard")
}
