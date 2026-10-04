package backendtest

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
	"github.com/wspl/demi/internal/commandpackage/file/fileproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/programtest"
)

// Built is a native package built by the workspace and published once for its
// owner's tests. Descriptor and Program are immutable after construction.
// Share this fixture across harnesses that use the same development release.
type Built struct {
	// Descriptor describes the built native package and its target artifacts.
	Descriptor commandproto.PackageDescriptor
	// Program is the path to the built native executable.
	Program   string
	directory string
	mu        sync.Mutex
	published chan struct{}
	catalog   *runners.NativeCatalog
	err       error
}

// BuildPackage obtains demi-file, demi-browser, demi-claude-code or the native
// fixture through programtest and describes its host-target artifact. Its test
// owns the release directory and catalog; start backends after this fixture.
func BuildPackage(ctx context.Context, t testing.TB, program string) (*Built, error) {
	t.Helper()
	path, err := programtest.Path(ctx, program)
	if err != nil {
		return nil, err
	}
	var fixture *remotehosttest.NativeFixture
	switch program {
	case "demi-file":
		fixture, err = remotehosttest.NewNativeFixture(ctx, fileproto.Package, path, fileproto.Operations())
	case "demi-browser":
		fixture, err = remotehosttest.NewNativeFixture(ctx, browserproto.Package, path, browserproto.OperationNames())
	case "demi-claude-code":
		var names []string
		for _, operation := range claudecodeproto.Operations() {
			names = append(names, string(operation))
		}
		fixture, err = remotehosttest.NewNativeFixture(ctx, claudecodeproto.Package, path, names)
	case "demi-native-fixture":
		fixture, err = remotehosttest.LoadNativeFixture(ctx)
	default:
		return nil, errors.New("unknown backend fixture program: " + program)
	}
	if err != nil {
		return nil, err
	}
	built := &Built{Descriptor: fixture.Descriptor, Program: path, directory: t.TempDir()}
	t.Cleanup(func() {
		built.mu.Lock()
		pending := built.published
		built.mu.Unlock()
		if pending != nil {
			<-pending
		}
		if built.catalog != nil {
			if err := built.catalog.Close(context.Background()); err != nil {
				t.Error(err)
			}
		}
	})
	return built, nil
}

// Catalog publishes this package through the real native-config entry point.
// Concurrent callers share one publication; cancellation ends their wait only.
func (b *Built) Catalog(ctx context.Context) (*runners.NativeCatalog, error) {
	b.mu.Lock()
	pending := b.published
	if pending == nil {
		pending = make(chan struct{})
		b.published = pending
		b.mu.Unlock()
		b.catalog, b.err = b.publish(ctx)
		close(pending)
	} else {
		b.mu.Unlock()
	}
	select {
	case <-pending:
		return b.catalog, b.err
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// publish writes the development release without a duplicate config contract.
func (b *Built) publish(ctx context.Context) (*runners.NativeCatalog, error) {
	for target := range b.Descriptor.Targets {
		directory := filepath.Join(b.directory, target)
		if err := os.MkdirAll(directory, 0o755); err != nil {
			return nil, err
		}
		name := "commands"
		if strings.Contains(target, "windows") {
			name += ".exe"
		}
		if err := os.Symlink(b.Program, filepath.Join(directory, name)); err != nil {
			return nil, err
		}
	}
	descriptor, err := contract.EncodeJSON(b.Descriptor)
	if err != nil {
		return nil, err
	}
	publication := artifacts.Publication{Mode: artifacts.CreateNew, Permissions: artifacts.Default}
	if err := artifacts.PublishBytes(
		ctx,
		filepath.Join(b.directory, "descriptor.json"),
		descriptor,
		publication,
	); err != nil {
		return nil, err
	}
	// A fixed fixture document, validated by runners' generated native decoder;
	// this is not a second Go declaration of the private nativeConfig shape.
	config, err := contract.EncodeJSON(
		json.RawMessage(`{"releases":[{"directory":".","executable":"commands"}],"store":{"provider":"local"}}`),
	)
	if err != nil {
		return nil, err
	}
	path := filepath.Join(b.directory, "native.json")
	if err := artifacts.PublishBytes(ctx, path, config, publication); err != nil {
		return nil, err
	}
	return runners.PublishNative(ctx, path)
}

// UsePackage binds this harness's native commands to an owned built release.
func (h *Harness) UsePackage(ctx context.Context, built *Built) error {
	catalog, err := built.Catalog(ctx)
	if err != nil {
		return err
	}
	h.Config.Native = catalog
	return nil
}

// UseNativeFixture also registers the runner fixture's six user streams.
func (h *Harness) UseNativeFixture(ctx context.Context, built *Built) error {
	if err := h.UsePackage(ctx, built); err != nil {
		return err
	}
	schema, err := commanddecl.NewSchema([]byte(`{}`))
	if err != nil {
		return err
	}
	var streams []plugin.Stream
	for _, name := range []string{"echo", "where", "retain", "held", "stall_release", "stalled"} {
		streams = append(
			streams,
			plugin.Stream{
				Name:      name,
				Operation: commanddecl.NativeOperation{Package: built.Descriptor.ID, Operation: name},
				Receives:  plugin.Schema{Schema: schema},
				Sends:     plugin.Schema{Schema: schema},
			},
		)
	}
	h.Config.Plugins = append(h.Config.Plugins, StreamsPlugin(streams))
	return nil
}
