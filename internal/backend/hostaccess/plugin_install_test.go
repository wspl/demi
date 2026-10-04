package hostaccess

import (
	"context"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/runnerproto"
)

// Cost: local storage and a scripted connection. Every filesystem reply is an
// event; no wall time is used to decide whether installation is cached.
func TestConnectedJobInstallsDirectoriesByRevisionAndConnection(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	blob, err := s.blobs.Put(t.Context(), []byte("tool bytes"))
	if err != nil {
		t.Fatal(err)
	}
	directory := plugin.HostDirectory{
		Name:  "tools",
		Files: []plugin.DirectoryFile{{Path: "bin/tool", Executable: true, Blob: blob}, {Path: "readme", Blob: blob}},
	}
	s.directories = DirectorySets{{Plugin: "test", Directories: []plugin.HostDirectory{directory}}}
	identity, err := ConversationHostForNode(t.Context(), s, record.ID)
	if err != nil {
		t.Fatal(err)
	}
	job := func() <-chan hostResult[struct{}] {
		return startHostOperation(t, func(ctx context.Context) (struct{}, error) {
			return struct{}{}, RunJob(ctx, s, record.ID, identity.Key(), func(context.Context) error {
				if held := s.conversations.Slot(record.ID).FileGate().TryReserve(); held != nil {
					held.Release()
					t.Error("job escaped file admission")
				}
				return nil
			})
		})
	}
	first := job()
	listing := nextHostMessage[*runnerproto.FSReaddir](t, r)
	base := "/home/test/.demi/plugins/test"
	if listing.Path != base {
		t.Fatal(listing.Path)
	}
	r.send(t, &runnerproto.FSOK{ID: listing.ID, Result: &runnerproto.FSReaddirResult{Value: []runnerproto.DirEntry{}}})
	partial := base + "/." + directory.HostName() + ".partial"
	stat := nextHostMessage[*runnerproto.FSLstat](t, r)
	if stat.Path != partial {
		t.Fatal(stat.Path)
	}
	r.send(t, &runnerproto.FSError{ID: stat.ID, Code: new("ENOENT"), Message: "missing"})
	mkdir := nextHostMessage[*runnerproto.FSMkdir](t, r)
	if mkdir.Path != partial {
		t.Fatal(mkdir.Path)
	}
	r.send(t, &runnerproto.FSOK{ID: mkdir.ID, Result: &runnerproto.FSMkdirResult{}})
	for _, file := range directory.Files {
		path, bytes := receiveHostWrite(t, s, r, device)
		if path != partial+"/"+file.Path || string(bytes) != "tool bytes" {
			t.Fatal(path, string(bytes))
		}
		chmod := nextHostMessage[*runnerproto.FSChmod](t, r)
		mode := uint32(0o444)
		if file.Executable {
			mode = 0o555
		}
		if chmod.Path != path || chmod.Mode != mode {
			t.Fatal(chmod)
		}
		r.send(t, &runnerproto.FSOK{ID: chmod.ID, Result: &runnerproto.FSChmodResult{}})
	}
	chmod := nextHostMessage[*runnerproto.FSChmod](t, r)
	if chmod.Path != partial+"/bin" || chmod.Mode != 0o555 {
		t.Fatal(chmod)
	}
	r.send(t, &runnerproto.FSOK{ID: chmod.ID, Result: &runnerproto.FSChmodResult{}})
	move := nextHostMessage[*runnerproto.FSMv](t, r)
	installed := base + "/" + directory.HostName()
	if move.Path != partial || move.Destination != installed {
		t.Fatal(move)
	}
	r.send(t, &runnerproto.FSOK{ID: move.ID, Result: &runnerproto.FSMvResult{}})
	chmod = nextHostMessage[*runnerproto.FSChmod](t, r)
	if chmod.Path != installed || chmod.Mode != 0o555 {
		t.Fatal(chmod)
	}
	r.send(t, &runnerproto.FSOK{ID: chmod.ID, Result: &runnerproto.FSChmodResult{}})
	if result := <-first; result.err != nil {
		t.Fatal(result.err)
	}
	// Completion without any message proves this revision is cached.
	cached := job()
	select {
	case result := <-cached:
		if result.err != nil {
			t.Fatal(result.err)
		}
	case <-r.outgoing:
		t.Fatal("same connection and revision reinstalled")
	}
	if err := r.connection.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	r = connectHost(t, s, device)
	reconnected := job()
	listing = nextHostMessage[*runnerproto.FSReaddir](t, r)
	r.send(
		t,
		&runnerproto.FSOK{
			ID: listing.ID,
			Result: &runnerproto.FSReaddirResult{
				Value: []runnerproto.DirEntry{{Name: directory.HostName(), IsDirectory: true}},
			},
		},
	)
	if result := <-reconnected; result.err != nil {
		t.Fatal(result.err)
	}
	// Disabling a plugin is a new revision, including removal of readonly files.
	s.directories = DirectorySets{{Plugin: "test"}}
	disabled := job()
	listing = nextHostMessage[*runnerproto.FSReaddir](t, r)
	r.send(
		t,
		&runnerproto.FSOK{
			ID: listing.ID,
			Result: &runnerproto.FSReaddirResult{
				Value: []runnerproto.DirEntry{{Name: directory.HostName(), IsDirectory: true}},
			},
		},
	)
	stat = nextHostMessage[*runnerproto.FSLstat](t, r)
	r.send(
		t,
		&runnerproto.FSOK{
			ID:     stat.ID,
			Result: &runnerproto.FSLstatResult{Value: runnerproto.FileStat{IsDirectory: true}},
		},
	)
	chmod = nextHostMessage[*runnerproto.FSChmod](t, r)
	if chmod.Path != installed || chmod.Mode != 0o755 {
		t.Fatal(chmod)
	}
	r.send(t, &runnerproto.FSOK{ID: chmod.ID, Result: &runnerproto.FSChmodResult{}})
	listing = nextHostMessage[*runnerproto.FSReaddir](t, r)
	r.send(t, &runnerproto.FSOK{ID: listing.ID, Result: &runnerproto.FSReaddirResult{Value: []runnerproto.DirEntry{}}})
	remove := nextHostMessage[*runnerproto.FSRm](t, r)
	if remove.Path != installed || remove.Recursive == nil || !*remove.Recursive {
		t.Fatal(remove)
	}
	r.send(t, &runnerproto.FSOK{ID: remove.ID, Result: &runnerproto.FSRmResult{}})
	if result := <-disabled; result.err != nil {
		t.Fatal(result.err)
	}
	if s.jobs != 4 {
		t.Fatal("job completion accounting", s.jobs)
	}
}

func TestConnectedPluginLookHasNoActivityAndTransitionEndsRead(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	result := startHostOperation(t, func(ctx context.Context) ([]plugin.HostFile, error) {
		return ReadFiles(ctx, s, record.ID, []plugin.HostRead{{Path: "/notes", Limit: 10}})
	})
	r.stat(t, 100)
	read := nextHostMessage[*runnerproto.FSReadFile](t, r)
	if read.Length == nil || *read.Length != 10 {
		t.Fatal(read)
	}
	r.send(t, &runnerproto.FSOK{ID: read.ID, Result: &runnerproto.FSReadFileResult{}})
	slot := s.conversations.Slot(record.ID)
	if !slot.FileGate().State().LastDemandEnd.IsZero() {
		t.Fatal("plugin look counted as demand")
	}
	hold, err := HoldForTransition(t.Context(), s, record.ID, nil)
	if err != nil {
		t.Fatal(err)
	}
	hold.Release()
	if completed := <-result; completed.err == nil {
		t.Fatal("transition did not end plugin read")
	}
	if s.idle < 2 {
		t.Fatal("plugin admission omitted idle watch")
	}
}

func TestConnectedPluginReadsKeepPathOrderAndPathFailuresLocal(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	result := startHostOperation(t, func(ctx context.Context) ([]plugin.HostFile, error) {
		return ReadFiles(
			ctx,
			s,
			record.ID,
			[]plugin.HostRead{
				{Path: "/missing"},
				{Path: "/denied"},
				{Path: "/dir"},
				{Path: "/other"},
				{Path: "/file", Limit: 5},
			},
		)
	})
	for _, code := range []string{"ENOENT", "EACCES"} {
		stat := nextHostMessage[*runnerproto.FSStat](t, r)
		r.send(t, &runnerproto.FSError{ID: stat.ID, Code: &code, Message: code})
	}
	stat := nextHostMessage[*runnerproto.FSStat](t, r)
	r.send(
		t,
		&runnerproto.FSOK{
			ID:     stat.ID,
			Result: &runnerproto.FSStatResult{Value: runnerproto.FileStat{IsDirectory: true}},
		},
	)
	listing := nextHostMessage[*runnerproto.FSReaddir](t, r)
	r.send(
		t,
		&runnerproto.FSOK{
			ID:     listing.ID,
			Result: &runnerproto.FSReaddirResult{Value: []runnerproto.DirEntry{{Name: "a", IsSymbolicLink: true}}},
		},
	)
	stat = nextHostMessage[*runnerproto.FSStat](t, r)
	r.send(t, &runnerproto.FSOK{ID: stat.ID, Result: &runnerproto.FSStatResult{Value: runnerproto.FileStat{}}})
	r.stat(t, 100)
	if path := sendHostRead(t, s, r, device, "short"); path != "/file" {
		t.Fatal(path)
	}
	completed := <-result
	if completed.err != nil {
		t.Fatal(completed.err)
	}
	wanted := []plugin.HostFile{
		&plugin.HostFileMissing{},
		&plugin.HostFileUnreadable{Message: "EACCES"},
		&plugin.HostFileDirectory{Entries: []plugin.HostEntry{{Name: "a", Kind: plugin.EntryKindSymlink}}},
		&plugin.HostFileOther{},
		&plugin.HostFileFile{Bytes: []byte("short"), Size: 100},
	}
	if diff := cmp.Diff(wanted, completed.value); diff != "" {
		t.Fatal(diff)
	}
	if !s.conversations.Slot(record.ID).FileGate().State().LastDemandEnd.IsZero() {
		t.Fatal("successful plugin read counted as activity")
	}
}
