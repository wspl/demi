package usershard

import (
	"context"
	"log/slog"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// ChannelSession identifies the authenticated session opening a page channel.
type ChannelSession struct {
	// Token identifies the authenticated browser session.
	Token database.TokenHash
	// User is the authenticated user’s page representation.
	User webapi.UserDTO
	// ExpiresAt records the session’s current expiry.
	ExpiresAt core.Timestamp
}

// ServeSyncChannel takes ownership of socket and serves initial product state
// and changes until the page, session or shard closes. It always closes socket.
func (s *Shard) ServeSyncChannel(
	ctx context.Context,
	socket *websocket.Conn,
	session ChannelSession,
) error {
	return s.serveSync(ctx, socket, session)
}

// ServeConversationSocket takes ownership of socket. An admitted agent frame
// finishes even during shutdown, with outbox delivery continuing until then.
// Socket ownership is registered before any wait and ends with its close.
func (s *Shard) ServeConversationSocket(
	ctx context.Context,
	conversation database.ConversationRecord,
	socket *websocket.Conn,
) error {
	return s.serveConversation(ctx, conversation, socket)
}

// Mark marks part changed on this user's open synchronization channels.
func (s *Shard) Mark(part pagesync.Part) {
	s.services.Sync.Mark(s.user, part)
}

// AdoptRunner takes and closes the socket of a runner presenting device's token.
func (s *Shard) AdoptRunner(
	ctx context.Context,
	device database.DeviceRecord,
	runner runnerwire.RunnerInfo,
	socket *runners.Socket,
) error {
	return s.adopt(ctx, device, runner, socket, nil)
}

// AdoptClaimed takes a newly paired runner's socket. It reports its bound DTO
// through bound before serving, or closes bound without a value if refused.
// The caller supplies a buffered channel of capacity one; adoption owns close.
func (s *Shard) AdoptClaimed(
	ctx context.Context,
	device database.DeviceRecord,
	runner runnerwire.RunnerInfo,
	socket *runners.Socket,
	bound chan<- webapi.DeviceDTO,
) error {
	return s.adopt(ctx, device, runner, socket, bound)
}

// RevokeDevice ends exposes, removes the device and attachments, and tells its
// runner it was revoked and must stop permanently.
func (s *Shard) RevokeDevice(ctx context.Context, device webapi.DeviceID) error {
	if err := s.stopExposes(ctx, device); err != nil {
		slog.ErrorContext(ctx, "the exposes of a device could not be destroyed: "+err.Error(), "device", device)
	}
	if err := s.Control().DeleteDevice(ctx, device); err != nil {
		return err
	}
	s.Mark(pagesync.Part{Kind: pagesync.Devices})
	s.devices.Revoke(device)
	return nil
}

// DeviceList lists the user's devices as the page sees them.
func (s *Shard) DeviceList(ctx context.Context) ([]webapi.DeviceDTO, error) {
	return s.devices.DeviceList(ctx, s.Control(), s.user)
}

// ExposeConnection is the network stream admitted for a relayed expose.
// The edge owns both pipe ends and defers releasing the host-access Lease.
// Its context stops copying on expose removal, shard shutdown or edge release.
type ExposeConnection struct {
	// ToService carries visitor bytes; closing it half-closes the network socket.
	ToService *remotehost.PipeWriter
	// FromService carries bytes until the service's end-of-stream.
	FromService *remotehost.PipeReader
	// Lease lasts until copying ends or the expose is removed.
	Lease *hostaccess.Lease
}

// OpenExposeConnection admits a connection and opens its network stream through
// device access, without a conversation, file gate or Cloud wake.
func (s *Shard) OpenExposeConnection(ctx context.Context, id webapi.ExposeID) (*ExposeConnection, error) {
	return shardCall(ctx, s, func(ctx context.Context) (*ExposeConnection, error) {
		return s.connectExpose(ctx, id)
	})
}
