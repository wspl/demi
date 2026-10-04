package backend

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/netip"
	"os"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/httpserver"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/webapiproto"
)

// Backend is a running backend. Its owner must call Close and wait for all
// services, shards, connections and storage to end, including after ctx ends.
type Backend struct {
	storage       *usershard.Storage
	services      *usershard.Services
	shards        *usershard.Shards
	edge          *httpserver.Server
	machines      *cloud.Client
	closeObjects  func() error
	cancel        context.CancelFunc
	stopDeaths    context.CancelFunc
	stopRetention context.CancelFunc
	retention     sync.WaitGroup
	deaths        sync.WaitGroup
	request       chan struct{}
	done          chan struct{}
	closeOnce     sync.Once
	closeErr      error
}

// Start starts the backend. It serves once this returns: the data directory
// and the instance secret, then the databases and the object store, the
// shared services and the shards, and the listener last. A failed start
// releases everything acquired by that attempt. ctx owns the backend lifetime.
func Start(ctx context.Context, config Config) (_ *Backend, err error) {
	secret, s3, err := startupStorageConfig(ctx, config)
	if err != nil {
		return nil, err
	}
	objects, err := blobs.Open(ctx, config.DataDir, s3)
	if err != nil {
		return nil, fmt.Errorf("the object store cannot be opened: %w", err)
	}
	// Parent cancellation requests ordered shutdown; it must not kill runner
	// connections before the shards have saved their Clouds.
	life, cancel := context.WithCancel(context.WithoutCancel(ctx))
	b := &Backend{closeObjects: objects.Close, cancel: cancel, request: make(chan struct{}), done: make(chan struct{})}
	defer func() {
		if err != nil {
			if failure := b.shutdown(context.WithoutCancel(ctx)); failure != nil {
				slog.Error("storage did not close after a failed start", "error", failure)
			}
		}
	}()
	var observed blobs.Objects = objects
	if config.ObserveObjects != nil {
		observed = config.ObserveObjects(objects)
	}
	b.storage, err = usershard.OpenStorage(ctx, config.DataDir, config.Clock, observed)
	if err != nil {
		return nil, fmt.Errorf("storage cannot be opened: %w", err)
	}
	deaths, err := b.startServices(life, config, secret)
	if err != nil {
		return nil, err
	}
	if err := b.recover(ctx, life, deaths); err != nil {
		return nil, err
	}
	state := httpserver.AppState{
		Services: b.services,
		Shards:   b.shards,
		Site:     &httpserver.Site{PublicURL: config.PublicURL, RunnerReleases: config.RunnerReleases},
	}
	b.edge, err = httpserver.Start(life, config.Address, state, config.WebDirectory)
	if err != nil {
		return nil, fmt.Errorf("the backend cannot listen on %s: %w", config.Address, err)
	}
	if config.Lifecycle.RetentionInterval != 0 {
		retentionCtx, stop := context.WithCancel(life)
		b.stopRetention = stop
		b.retention.Go(func() {
			// The scheduler owns its per-pass diagnostics; cancellation ends scheduling.
			_ = usershard.ScheduleRetention(retentionCtx, b.services, b.shards)
		})
	}
	go func() {
		select {
		case <-ctx.Done():
		case <-b.request:
		}
		b.closeErr = b.shutdown(context.WithoutCancel(ctx))
		close(b.done)
	}()
	return b, nil
}

// LocalAddr is the address the listener is bound to.
func (b *Backend) LocalAddr() netip.AddrPort { return b.edge.LocalAddr() }

// Close requests ordered shutdown and joins it. A canceled cleanup context
// ends only this wait; a later Close with a live context joins the same shutdown.
func (b *Backend) Close(ctx context.Context) error {
	b.closeOnce.Do(func() { close(b.request) })
	select {
	case <-b.done:
		return b.closeErr
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Services borrows shared handles for test support; ownership stays with Backend.
func (b *Backend) Services() *usershard.Services { return b.services }

// Shards borrows user routing for test support; ownership stays with Backend.
func (b *Backend) Shards() *usershard.Shards { return b.shards }

// shutdown drains the backend in dependency order, also after partial startup.
func (b *Backend) shutdown(ctx context.Context) error {
	var failures []any
	record := func(err error) {
		if err != nil {
			failures = append(failures, err)
		}
	}
	if b.edge != nil {
		b.edge.StopAccepting()
	}
	if b.stopRetention != nil {
		b.stopRetention()
	}
	if b.services != nil {
		record(b.services.Logins.Close(ctx))
	}
	if b.shards != nil {
		if err := b.shards.Close(ctx); err != nil {
			record(fmt.Errorf("a Cloud was not saved: %w", err))
		}
	}
	b.retention.Wait()
	if b.services != nil {
		b.services.Claims.Close()
	}
	// Shards no longer need runner connections or death routing.
	if b.stopDeaths != nil {
		b.stopDeaths()
	}
	b.deaths.Wait()
	if b.machines != nil {
		if err := b.machines.Close(ctx); err != nil {
			record(fmt.Errorf("the machine manager did not reconcile: %w", err))
		}
	}
	if b.edge != nil {
		if err := b.edge.Close(ctx); err != nil {
			record(fmt.Errorf("the listener did not stop cleanly: %w", err))
		}
	}
	if b.services != nil {
		record(b.services.Close(ctx))
	}
	if b.storage != nil {
		record(b.storage.Close(ctx))
	}
	record(b.closeObjects())
	b.cancel()
	if len(failures) == 0 {
		return nil
	}
	// Every failure stays visible to errors.Is and errors.As, in shutdown order.
	return fmt.Errorf("shutdown failed: %w"+strings.Repeat("; %w", len(failures)-1), failures...)
}

// startupStorageConfig resolves the secret and object store before opening storage.
func startupStorageConfig(ctx context.Context, config Config) (*InstanceSecret, *blobs.S3Config, error) {
	// An empty data directory is the working directory: nothing is created, and
	// the relative paths below address it.
	if config.DataDir != "" {
		if err := os.MkdirAll(config.DataDir, 0o755); err != nil {
			return nil, nil, fmt.Errorf("the data directory %s cannot be created: %w", config.DataDir, err)
		}
	}
	secret := config.InstanceSecret
	if secret == nil {
		value, err := loadSecret(ctx, config.DataDir)
		if err != nil {
			return nil, nil, err
		}
		secret = &value
	}
	var s3 *blobs.S3Config
	if config.ObjectStore != "" || config.objectStoreSet {
		value, err := blobs.ReadS3Config(ctx, config.ObjectStore)
		if err != nil {
			return nil, nil, fmt.Errorf("DEMI_OBJECT_STORE_CONFIG cannot be used: %w", err)
		}
		s3 = &value
	}
	return secret, s3, nil
}

// startServices composes the shared services and shard routing after storage opens.
func (b *Backend) startServices(
	life context.Context,
	config Config,
	secret *InstanceSecret,
) (<-chan webapiproto.DeviceID, error) {
	keys, err := secret.serviceKeys()
	if err != nil {
		return nil, err
	}
	machines, deaths := cloud.NewClient(life, config.MachinesSocket)
	b.machines = machines
	setup := usershard.ProviderSetup{
		Families:       config.Families,
		ModelsDevURL:   config.ModelsDevURL,
		ClaudeReleases: config.ClaudeReleases,
		Logins:         config.Logins,
		Clock:          config.Clock,
	}
	settings := usershard.ServiceSettings{
		Mode:          config.Mode,
		Mail:          config.AccountMail,
		Runners:       config.Runners,
		Conversations: config.Conversations,
		Pages:         config.Pages,
		Native:        config.Native,
		Plugins:       config.Plugins,
		Cloud:         cloud.NewServices(machines, config.Cloud),
		Lifecycle:     config.Lifecycle,
		ExposeDomain:  config.ExposeDomain,
		Exposes:       config.Exposes,
	}
	b.services, err = usershard.StartServices(life, b.storage, keys, setup, settings)
	if err != nil {
		return nil, err
	}
	b.services.Hooks = config.Hooks
	b.shards, err = usershard.NewShards(life, b.services)
	if err != nil {
		return nil, fmt.Errorf("the shard threads cannot start: %w", err)
	}
	return deaths, nil
}

// recover routes Cloud deaths before recovering saved resets, forks and wakeups.
func (b *Backend) recover(ctx, life context.Context, deaths <-chan webapiproto.DeviceID) error {
	deathCtx, stopDeaths := context.WithCancel(life)
	b.stopDeaths = stopDeaths
	b.deaths.Go(func() {
		// Cancellation is the expected termination of this owner task.
		if err := usershard.RouteDeaths(
			deathCtx,
			deaths,
			b.services,
			b.shards,
		); err != nil &&
			!errors.Is(err, context.Canceled) {
			slog.Error(fmt.Sprintf("the death of a Cloud could not be routed: %v", err))
		}
	})
	if err := cloud.RecoverResets(ctx, b.services.Control, b.services.Cloud); err != nil {
		return fmt.Errorf("the Clouds cannot be recovered: %w", err)
	}
	if err := usershard.RecoverForks(ctx, b.services.Control, b.services.Conversations); err != nil {
		return fmt.Errorf("storage cannot be opened: %w", err)
	}
	if err := usershard.RearmWakeups(ctx, b.services.Control, b.shards); err != nil {
		slog.Error("the saved wakeups cannot be listed", "error", err)
	}
	return nil
}
