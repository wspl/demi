package pluginhost_test

import (
	"context"
	"encoding/json"
	"errors"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/pluginhost"
	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

type fakePlugin struct {
	call  func(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error)
	close func()
}

func (p *fakePlugin) Call(
	ctx context.Context,
	request plugin.Request,
	port plugin.Port,
) (plugin.Reply, error) {
	if p.call != nil {
		return p.call(ctx, request, port)
	}
	switch request.(type) {
	case *plugin.RequestCommand:
		return &plugin.ReplyExit{}, nil
	case *plugin.RequestPanelTab, *plugin.RequestTopic:
		return &plugin.ReplyDone{}, nil
	case *plugin.RequestContext:
		return &plugin.ReplyContext{Text: new("news")}, nil
	case *plugin.RequestPageState:
		return &plugin.ReplyState{State: json.RawMessage(`{}`)}, nil
	case *plugin.RequestPageCall:
		return &plugin.ReplyResult{Result: json.RawMessage(`{}`)}, nil
	}
	return nil, errors.New("unexpected fake plugin request")
}

func (p *fakePlugin) Close() {
	if p.close != nil {
		p.close()
	}
}

type fakeFactory struct {
	manifest plugin.Manifest
	make     func() plugin.Plugin
}

func (f *fakeFactory) Manifest() plugin.Manifest {
	return f.manifest
}

func (f *fakeFactory) Instance() plugin.Plugin {
	if f.make != nil {
		return f.make()
	}
	return &fakePlugin{}
}

// manifest declares a fake plugin's page, both state scopes and both method scopes.
func manifest(t *testing.T, id string) plugin.Manifest {
	t.Helper()
	schema, err := commanddecl.NewSchema(
		[]byte(`{"type":"object","properties":{"text":{"type":"string"}},"additionalProperties":false}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	s := plugin.Schema{Schema: schema}
	return plugin.Manifest{
		ID:          plugin.ID(id),
		Name:        "Name " + id,
		Description: "Description " + id,
		Context:     true,
		Page: &plugin.Page{
			Package:      "@test/" + id,
			User:         &plugin.State{Schema: s, Topics: []plugin.Topic{plugin.TopicExposes}},
			Conversation: &plugin.State{Schema: s, Topics: []plugin.Topic{plugin.TopicJobs}},
			Methods: []plugin.Method{
				{
					Name:   "user",
					Scope:  plugin.ScopeUser,
					Params: s,
					Result: s,
				},
				{
					Name:   "conversation",
					Scope:  plugin.ScopeConversation,
					Params: s,
					Result: s,
				},
			},
		},
	}
}

// command declares one group with one leaf for a fake plugin.
func command(name string, placement plugin.Placement, operation *commanddecl.NativeOperation) plugin.Commands {
	var kind commanddecl.LeafKind[commanddecl.NativeOperation] = &commanddecl.RPC[commanddecl.NativeOperation]{}
	if operation != nil {
		kind = &commanddecl.Native[commanddecl.NativeOperation]{Binding: *operation}
	}
	return plugin.Commands{
		Placement: placement,
		Tree: plugin.Declaration{
			Node: &commanddecl.Group[commanddecl.NativeOperation]{
				Name:    name,
				Summary: "A group.",
				Subcommands: []commanddecl.Node[commanddecl.NativeOperation]{
					&commanddecl.Leaf[commanddecl.NativeOperation]{Name: "run", Summary: "Run.", Kind: kind},
				},
			},
		},
	}
}

// registry builds fake plugins against a catalog serving every declared operation.
func registry(t *testing.T, factories ...plugin.Factory) *pluginhost.Registry {
	t.Helper()
	r, err := pluginhost.NewRegistry(factories, func(commanddecl.NativeOperation) bool {
		return true
	})
	if err != nil {
		t.Fatal(err)
	}
	return r
}

type fakeShard struct {
	control *database.ControlService
	sync    pagesync.Registry
	user    webapiproto.UserID
	// Only CommitUses is reached by the plugin host; Media belongs to the agent
	// store and deliberately has no fake here.
	uses        fakeUses
	packageCall func(
		context.Context,
		webapiproto.ConversationID,
		commanddecl.NativeOperation,
		json.RawMessage,
		plugin.CallKind,
	) (json.RawMessage, error)
	hosts   func(context.Context, webapiproto.ConversationID) ([]plugin.ConversationHost, error)
	files   func(context.Context, webapiproto.ConversationID, []plugin.HostRead) ([]plugin.HostFile, error)
	put     func(context.Context, types.B64Bytes) (types.BlobRef, error)
	blob    func(context.Context, types.BlobRef) (*types.B64Bytes, error)
	exposes func(context.Context) (plugin.ExposeList, error)
	create  func(context.Context, webapiproto.DeviceID, string, uint64) (plugin.ExposeRecord, error)
	renew   func(context.Context, webapiproto.ExposeID, uint64) (plugin.ExposeRecord, error)
	remove  func(context.Context, webapiproto.ExposeID) error
}

type fakeUses struct {
	database.OwnerBlobs
	mu      sync.Mutex
	touched []types.BlobRef
	fail    error
}

func (u *fakeUses) CommitUses(_ context.Context, blobs []types.BlobRef) error {
	u.mu.Lock()
	defer u.mu.Unlock()
	u.touched = append(u.touched, blobs...)
	return u.fail
}

func (s *fakeShard) Control() *database.ControlService {
	return s.control
}

func (s *fakeShard) Marks() pagesync.UserMarks {
	return s.sync.Of(s.user)
}

func (s *fakeShard) BlobUses() database.OwnerBlobs {
	return &s.uses
}

func (s *fakeShard) PackageCall(
	ctx context.Context,
	c webapiproto.ConversationID,
	operation commanddecl.NativeOperation,
	args json.RawMessage,
	kind plugin.CallKind,
) (json.RawMessage, error) {
	return s.packageCall(ctx, c, operation, args, kind)
}

func (s *fakeShard) ConversationHosts(
	ctx context.Context,
	c webapiproto.ConversationID,
) ([]plugin.ConversationHost, error) {
	return s.hosts(ctx, c)
}

func (s *fakeShard) ReadHostFiles(
	ctx context.Context,
	c webapiproto.ConversationID,
	reads []plugin.HostRead,
) ([]plugin.HostFile, error) {
	return s.files(ctx, c, reads)
}

func (s *fakeShard) PutBlob(ctx context.Context, bytes types.B64Bytes) (types.BlobRef, error) {
	return s.put(ctx, bytes)
}

func (s *fakeShard) Blob(ctx context.Context, ref types.BlobRef) (*types.B64Bytes, error) {
	return s.blob(ctx, ref)
}

func (s *fakeShard) Exposes(ctx context.Context) (plugin.ExposeList, error) {
	return s.exposes(ctx)
}

func (s *fakeShard) CreateExpose(
	ctx context.Context,
	d webapiproto.DeviceID,
	address string,
	lifetime uint64,
) (plugin.ExposeRecord, error) {
	return s.create(ctx, d, address, lifetime)
}

func (s *fakeShard) RenewExpose(
	ctx context.Context,
	id webapiproto.ExposeID,
	lifetime uint64,
) (plugin.ExposeRecord, error) {
	return s.renew(ctx, id, lifetime)
}

func (s *fakeShard) RemoveExpose(ctx context.Context, id webapiproto.ExposeID) error {
	return s.remove(ctx, id)
}

// userFixture owns a real control store and a fake product boundary, with no Host.
func userFixture(t *testing.T, r *pluginhost.Registry) (*pluginhost.User, *fakeShard) {
	t.Helper()
	s := &fakeShard{
		control: databasetest.Control(t.Context(), t, types.SystemClock{}),
	}
	s.user = databasetest.Master(t.Context(), t, s.control).ID
	u := newUser(t, r, s)
	return u, s
}

// newUser registers host cleanup after the fixture database cleanup.
func newUser(t *testing.T, r *pluginhost.Registry, shard *fakeShard) *pluginhost.User {
	t.Helper()
	u := pluginhost.NewUser(r, shard.user, shard)
	t.Cleanup(func() {
		if err := u.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return u
}
