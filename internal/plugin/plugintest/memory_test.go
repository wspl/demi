package plugintest_test

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"slices"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

type scenario func(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error)

func (s scenario) Call(ctx context.Context, r plugin.Request, p plugin.Port) (plugin.Reply, error) {
	return s(ctx, r, p)
}

// TestStorageThroughLoopback protects compare-and-set, blob retention and
// detached JSON values through the same transport used by plugins' tests.
func TestStorageThroughLoopback(t *testing.T) {
	d := plugintest.New()
	p := plugintest.Loopback(scenario(func(ctx context.Context, _ plugin.Request, p plugin.Port) (plugin.Reply, error) {
		blob, err := p.PutBlob(ctx, types.B64Bytes("payload"))
		if err != nil {
			return nil, err
		}
		body := json.RawMessage(`{"z":"<>&\u2028","a":2}`)
		revision, err := p.WriteValueNaming(ctx, "key", body, nil, []types.BlobRef{blob})
		if err != nil {
			return nil, err
		}
		if revision != 1 {
			t.Fatalf("first revision = %d", revision)
		}
		_, err = p.WriteValue(ctx, "key", body, nil)
		var conflict *plugin.PortRefusalConflict
		if !errors.As(err, &conflict) {
			t.Fatalf("duplicate create = %v", err)
		}
		v, found, err := p.Value(ctx, "key")
		if err != nil {
			return nil, err
		}
		if !found || v.Revision != 1 {
			t.Fatalf("stored = %#v", v)
		}
		blobs := d.ValueBlobs("key")
		if !slices.Equal(blobs, []types.BlobRef{blob}) {
			t.Fatalf("retained = %v", blobs)
		}
		content, found, err := p.Blob(ctx, blob)
		if err != nil {
			return nil, err
		}
		if !found || string(content) != "payload" {
			t.Fatalf("blob = %v", content)
		}
		content[0] = 'X'
		if stored, _ := d.BlobBytes(blob); string(stored) != "payload" {
			t.Fatal("read mutated stored blob")
		}
		revision, err = p.WriteValue(ctx, "key", json.RawMessage(`null`), &revision)
		if err != nil {
			return nil, err
		}
		if revision != 2 || len(d.ValueBlobs("key")) != 0 {
			t.Fatal("replacement retained old revision or blobs")
		}
		values, err := p.Values(ctx)
		if err != nil {
			return nil, err
		}
		if len(values) != 1 || !bytes.Equal(values["key"].Value, []byte("null")) {
			t.Fatalf("values = %#v", values)
		}
		if err := p.RemoveValue(ctx, "key", 1); !errors.As(err, &conflict) {
			t.Fatalf("stale remove = %v", err)
		}
		if err := p.RemoveValue(ctx, "key", revision); err != nil {
			return nil, err
		}
		v, found, err = p.Value(ctx, "key")
		if err != nil {
			return nil, err
		}
		if found {
			t.Fatal("removed value remains")
		}
		return &plugin.ReplyResult{Result: json.RawMessage(`{"done":true}`)}, nil
	}))
	_, err := p.Call(t.Context(), &plugin.RequestPageState{User: validUser(t)}, d.Port())
	if err != nil {
		t.Fatal(err)
	}
}

// TestHostFilesAndDirectories covers stopped Hosts, truncation, implicit
// directories and the content-addressed installation path without processes.
func TestHostFilesAndDirectories(t *testing.T) {
	d := plugintest.New()
	p := d.Port()
	_, err := p.ReadHostFiles(t.Context(), []plugin.HostRead{{Path: "/a", Limit: 2}})
	var stopped *plugin.PortRefusalNotRunning
	if !errors.As(err, &stopped) {
		t.Fatalf("stopped Host = %v", err)
	}
	d.HostFiles = map[string][]byte{"/a/file": []byte("1234"), "/a/child/file": []byte("x")}
	files, err := p.ReadHostFiles(
		t.Context(),
		[]plugin.HostRead{{Path: "/a", Limit: 0}, {Path: "/a/file", Limit: 2}, {Path: "/missing", Limit: 9}},
	)
	if err != nil {
		t.Fatal(err)
	}
	dir, ok := files[0].(*plugin.HostFileDirectory)
	if !ok || len(dir.Entries) != 2 || dir.Entries[0].Name != "child" ||
		dir.Entries[0].Kind != plugin.EntryKindDirectory {
		t.Fatalf("directory = %#v", files[0])
	}
	file, ok := files[1].(*plugin.HostFileFile)
	if !ok || string(file.Bytes) != "12" || file.Size != 4 {
		t.Fatalf("file = %#v", files[1])
	}
	if _, ok := files[2].(*plugin.HostFileMissing); !ok {
		t.Fatalf("missing = %#v", files[2])
	}
	blob, err := p.PutBlob(t.Context(), types.B64Bytes("x"))
	if err != nil {
		t.Fatal(err)
	}
	dirs := []plugin.HostDirectory{{Name: "skill", Files: []plugin.DirectoryFile{{Path: "SKILL.md", Blob: blob}}}}
	paths, err := p.SetDirectories(t.Context(), dirs)
	if err != nil {
		t.Fatal(err)
	}
	if len(paths) != 1 || paths[0].Path != dirs[0].Path(d.Plugin) {
		t.Fatalf("paths = %v", paths)
	}
	dirs[0].Files[0].Path = "changed"
	if d.Directories()[0].Files[0].Path != "SKILL.md" {
		t.Fatal("stored directory aliases caller")
	}
}

// TestExposes uses a manually advanced clock for create, renew, expiry,
// availability and device-stop semantics; it never waits for real time.
func TestExposes(t *testing.T) {
	d := plugintest.New()
	device, err := webapiproto.ParseDeviceID("aaaaaaaaaaaaaaaaaaaaaaaaaa")
	if err != nil {
		t.Fatal(err)
	}
	d.Hosts = []plugin.ConversationHost{{Name: "Cloud", Device: device, Role: plugin.HostRoleMain, Online: true}}
	p := d.Port()
	first, err := p.CreateExpose(t.Context(), device, "8080", 10)
	if err != nil {
		t.Fatal(err)
	}
	if string(first.Address) != "127.0.0.1:8080" || first.ID != "aaaaaaaaaaaaaaaaaaaaaaaaab" {
		t.Fatalf("expose = %#v", first)
	}
	second, err := p.CreateExpose(t.Context(), device, "localhost:9000", 5)
	if err != nil {
		t.Fatal(err)
	}
	list, err := p.Exposes(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if len(list.Exposes) != 2 || list.Exposes[0].ID != second.ID {
		t.Fatalf("expiry order = %v", list.Exposes)
	}
	d.Now = second.ExpiresAt
	_, err = p.RenewExpose(t.Context(), second.ID, 20)
	var refusal *plugin.PortRefusalExpose
	if !errors.As(err, &refusal) || refusal.Reason != plugin.ExposeRefusalNotFound {
		t.Fatalf("renew expired = %v", err)
	}
	renewed, err := p.RenewExpose(t.Context(), first.ID, 20)
	if err != nil {
		t.Fatal(err)
	}
	if renewed.ExpiresAt <= first.ExpiresAt {
		t.Fatal("renew did not extend")
	}
	d.EndExposesOn(device)
	if len(d.LiveExposes()) != 0 {
		t.Fatal("device stop left exposes")
	}
	d.ExposesAvailable = false
	_, err = p.CreateExpose(t.Context(), device, "8080", 10)
	if !errors.As(err, &refusal) || refusal.Reason != plugin.ExposeRefusalUnavailable {
		t.Fatalf("unavailable = %v", err)
	}
}

// TestUntilAndPackageCalls checks event-driven waiting and callback reentry
// without a lock or goroutine escaping its owner.
func TestUntilAndPackageCalls(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		d := plugintest.New()
		d.PackageCalls = func(
			_ context.Context,
			_ cmddecl.NativeOperation,
			_ json.RawMessage,
			_ plugin.CallKind,
		) (json.RawMessage, error) {
			_ = d.Changes()
			return json.RawMessage(`{"ok":true}`), nil
		}
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		done := make(chan error, 1)
		go func() {
			done <- d.Until(ctx, func(d *plugintest.TestDemi) bool {
				return d.Changes() == 1
			})
		}()
		synctest.Wait()
		if err := d.Port().Changed(ctx, plugin.ScopeUser); err != nil {
			t.Fatal(err)
		}
		if err := <-done; err != nil {
			t.Fatal(err)
		}
		if _, err := d.Port().
			PackageCall(ctx, cmddecl.NativeOperation{
				Package:   "test",
				Operation: "read",
			}, json.RawMessage(`{}`), plugin.CallKindLooks); err != nil {
			t.Fatal(err)
		}
		if len(d.Called()) != 1 {
			t.Fatal("package call was not recorded")
		}
		go func() {
			done <- d.Until(ctx, func(*plugintest.TestDemi) bool {
				return false
			})
		}()
		synctest.Wait()
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("wait cancellation = %v", err)
		}
	})
}

func validUser(t *testing.T) webapiproto.UserID {
	t.Helper()
	user, err := webapiproto.ParseUserID("aaaaaaaaaaaaaaaaaaaaaaaaaa")
	if err != nil {
		t.Fatal(err)
	}
	return user
}
