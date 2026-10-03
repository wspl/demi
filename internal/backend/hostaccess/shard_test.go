package hostaccess

import (
	"context"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// testShard composes real storage and runner owners, with controlled product
// callbacks. Tests may block a callback with a channel, never with a time delay.
type testShard struct {
	mu            sync.Mutex
	owner         webapi.UserID
	control       *database.ControlService
	conversations *Conversations
	devices       runners.Devices
	pipes         *remotehost.Pipes
	commands      runners.CommandRouter
	blobs         *blobs.Namespace
	stores        *database.ConversationStores
	native        *runners.NativeCatalog
	installs      *PluginInstalls
	directories   DirectorySets
	idle          int
	jobs          int
	idleHook      func(webapi.ConversationID)
	directoryHook func(context.Context) (DirectorySets, error)
}

func newTestShard(t *testing.T) *testShard {
	t.Helper()
	s := &testShard{}
	s.control = databasetest.Control(t.Context(), t, core.SystemClock{})
	user := databasetest.Master(t.Context(), t, s.control)
	s.owner = user.ID
	s.stores = databasetest.Conversations(t.Context(), t, 4)
	bucket, err := blobs.Open(t.Context(), t.TempDir(), nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := bucket.Close(); err != nil {
			t.Error(err)
		}
	})
	s.blobs = blobs.New(bucket, core.SystemClock{}).ForUser(s.owner)
	s.pipes = remotehost.NewPipes(remotehost.Arrival)
	t.Cleanup(func() {
		if err := s.pipes.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	s.native = runners.UnpublishedCatalog()
	s.conversations = NewConversations(context.Background(), &s.mu)
	s.installs = NewPluginInstalls(&s.mu)
	t.Cleanup(func() {
		if err := s.conversations.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return s
}
func (s *testShard) User() webapi.UserID               { return s.owner }
func (s *testShard) Control() *database.ControlService { return s.control }
func (s *testShard) Clock() core.Clock                 { return core.SystemClock{} }
func (s *testShard) Devices() *runners.Devices         { return &s.devices }
func (s *testShard) Pipes() *remotehost.Pipes          { return s.pipes }
func (s *testShard) Commands() *runners.CommandRouter  { return &s.commands }
func (s *testShard) Conversations() *Conversations     { return s.conversations }
func (s *testShard) Blobs() *blobs.Namespace           { return s.blobs }
func (s *testShard) ConversationDB(id webapi.ConversationID) *database.ConversationDB {
	return s.stores.DB(id)
}
func (s *testShard) Native() *runners.NativeCatalog { return s.native }
func (s *testShard) PublicURL() *runners.PublicURL  { return nil }
func (s *testShard) CloudShard() cloud.CloudShard   { return nil }
func (s *testShard) TrackIdle(id webapi.ConversationID) {
	s.mu.Lock()
	s.idle++
	hook := s.idleHook
	s.mu.Unlock()
	if hook != nil {
		hook(id)
	}
}

func (s *testShard) JobEnded(webapi.ConversationID) {
	s.mu.Lock()
	s.jobs++
	s.mu.Unlock()
}

func (s *testShard) DirectorySets(ctx context.Context) (DirectorySets, error) {
	if s.directoryHook != nil {
		return s.directoryHook(ctx)
	}
	return s.directories, nil
}
func (s *testShard) PluginInstalls() *PluginInstalls { return s.installs }

func (s *testShard) conversation(t *testing.T) database.ConversationRecord {
	t.Helper()
	id := webapi.ConversationID("00000000-0000-4000-8000-000000000001")
	if _, _, err := s.control.CreateConversation(t.Context(), s.owner, id); err != nil {
		t.Fatal(err)
	}
	record, _, err := s.control.Conversation(t.Context(), id)
	if err != nil {
		t.Fatal(err)
	}
	return record
}

func (s *testShard) paired(t *testing.T, name string) database.DeviceRecord {
	t.Helper()
	record, err := s.control.CreateDevice(
		t.Context(),
		s.owner,
		name,
		runnerwire.RunnerPlatform("linux"),
		database.HashToken(name),
	)
	if err != nil {
		t.Fatal(err)
	}
	return record
}

func (s *testShard) target(
	t *testing.T,
	record database.ConversationRecord,
	device database.DeviceRecord,
	path string,
) database.ConversationRecord {
	t.Helper()
	if err := SwitchTarget(
		t.Context(),
		s,
		record,
		&webapi.ConversationTargetDevice{DeviceID: device.ID, Path: path},
	); err != nil {
		t.Fatal(err)
	}
	updated, err := OwnedConversation(t.Context(), s, record.ID)
	if err != nil {
		t.Fatal(err)
	}
	return updated
}
