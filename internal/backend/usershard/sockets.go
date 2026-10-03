package usershard

//revive:disable:unused-parameter

import (
	"context"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// ChannelSession identifies the authenticated session opening a page channel.
type ChannelSession struct {
	Token     database.TokenHash
	User      webapi.UserDTO
	ExpiresAt core.Timestamp
}

// ServeSyncChannel takes ownership of socket and serves initial product state
// and changes until the page, session or shard closes. It always closes socket.
func (s *Shard) ServeSyncChannel(ctx context.Context, socket *websocket.Conn, session ChannelSession) error {
	panic("not written: b-usershard")
}

// ServeConversationSocket takes ownership of socket. An admitted agent frame
// finishes even during shutdown, with outbox delivery continuing until then.
// Socket ownership is registered before any wait and ends with its close.
func (s *Shard) ServeConversationSocket(ctx context.Context, conversation database.ConversationRecord, socket *websocket.Conn) error {
	panic("not written: b-usershard")
}

// Mark marks part changed on this user's open synchronization channels.
func (s *Shard) Mark(part pagesync.Part) { panic("not written: b-usershard") }

// AdoptRunner takes and closes the socket of a runner presenting device's token.
func (s *Shard) AdoptRunner(ctx context.Context, device database.DeviceRecord, runner runnerwire.RunnerInfo, socket *websocket.Conn) error {
	panic("not written: b-usershard")
}

// AdoptClaimed takes a newly paired runner's socket. It reports its bound DTO
// through bound before serving, or closes bound without a value if refused.
// The caller supplies a buffered channel of capacity one; adoption owns close.
func (s *Shard) AdoptClaimed(ctx context.Context, device database.DeviceRecord, runner runnerwire.RunnerInfo, socket *websocket.Conn, bound chan<- webapi.DeviceDTO) error {
	panic("not written: b-usershard")
}

// RevokeDevice ends exposes, removes the device and attachments, and tells its
// runner it was revoked and must stop permanently.
func (s *Shard) RevokeDevice(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-usershard")
}

// DeviceList lists the user's devices as the page sees them.
func (s *Shard) DeviceList(ctx context.Context) ([]webapi.DeviceDTO, error) {
	panic("not written: b-usershard")
}

// ExposeConnection is the network stream admitted for a relayed expose.
// The edge owns both pipe ends and defers releasing Lease while copying bytes.
type ExposeConnection struct {
	// ToService carries visitor bytes; closing it half-closes the network socket.
	ToService *remotehost.PipeWriter
	// FromService carries bytes until the service's end-of-stream.
	FromService *remotehost.PipeReader
	// Lease lasts until copying ends or the expose is removed.
	Lease *expose.RelayAdmission
}

// OpenExposeConnection admits a connection and opens its network stream through
// device access, without a conversation, file gate or Cloud wake.
func (s *Shard) OpenExposeConnection(ctx context.Context, id webapi.ExposeID) (*ExposeConnection, error) {
	panic("not written: b-usershard")
}
