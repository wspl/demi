//go:build linux

package machines

import (
	"context"
	"errors"
	"log/slog"
	"net"
	"os"

	"github.com/wspl/demi/internal/machines/sandbox"
	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
)

// Run recovers and serves until cancellation, then drains every owned operation.
func Run(ctx context.Context, config Config) (err error) {
	if err = RequireRoot(); err != nil {
		return err
	}
	if err = RequirePrivateNamespace(); err != nil {
		return err
	}
	tools, err := system.Resolve(ctx, config.Runsc)
	if err != nil {
		return err
	}
	core, err := NewCore(config, tools)
	if err != nil {
		return err
	}
	if err = RequireRunsc(ctx, core); err != nil {
		return err
	}
	for _, path := range []string{config.Data, RuntimeDirectory} {
		if err = storage.CreatePrivate(ctx, path); err != nil {
			return err
		}
	}
	if config.Mode == ModeRecoverNamespace {
		return recoverInherited(ctx, core)
	}
	lock, err := AcquireLock(config.Data, RuntimeDirectory)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, lock.Close()) }()
	namespace := sandbox.NewSavedNamespace(RuntimeDirectory, config.Data)
	if err = namespace.Recover(ctx, lock.Data(), os.Args[1:]); err != nil {
		return err
	}
	if err = ReleaseProbes(ctx, config.Data); err != nil {
		return err
	}
	if config.Limits != nil {
		if err = sandbox.PrepareCgroups(ctx); err != nil {
			return err
		}
	} else {
		slog.Warn("demi-machine-manager: DEMI_MANAGED_LIMITS=off: Clouds run without CPU, memory or PID limits")
	}
	if err = RequireOneFilesystem(ctx, config.Working(), config.Images()); err != nil {
		return err
	}
	if err = FenceAndSave(ctx, core); err != nil {
		return err
	}
	if config.Mode == ModeRecover {
		return nil
	}
	return serveManager(ctx, core, namespace)
}

// notifyReady publishes the manager's readiness to systemd when configured.
func notifyReady(ctx context.Context) error {
	path := os.Getenv("NOTIFY_SOCKET")
	if path == "" {
		return nil
	}
	conn, err := (&net.Dialer{}).DialContext(ctx, "unixgram", path)
	if err != nil {
		return err
	}
	defer func() { _ = conn.Close() }() // Datagram close has no pending application data.
	_, err = conn.Write([]byte("READY=1"))
	return err
}

func serveManager(ctx context.Context, core *Core, namespace *sandbox.SavedNamespace) (err error) {
	config, tools := core.Config, core.Tools
	if err = core.Network.Prepare(ctx); err != nil {
		return err
	}
	base, err := storage.ImportBase(ctx, tools, config.Image, core.Store.Bases())
	if err != nil {
		return err
	}
	if err = namespace.Pin(ctx); err != nil {
		return err
	}
	if err = ProbeStorage(ctx, core); err != nil {
		return err
	}
	socket, err := BindSocket(ctx, config.Socket)
	if err != nil {
		return err
	}
	if err = notifyReady(ctx); err != nil {
		_ = socket.listener.Close()
		return err
	}
	deaths := make(chan machinewire.DeviceID, 64)
	manager := NewManager(core, base, deaths)
	limits := "on"
	if config.Limits == nil {
		limits = "off"
	}
	slog.Info("demi-machine-manager: gVisor/systrap ready at " + config.Socket + ", resource limits " + limits)
	flight := Serve(ctx, socket, manager, deaths)
	cleanup := context.WithoutCancel(ctx)
	drained := manager.Close(cleanup)
	flight.Wait(cleanup)
	if drained != nil {
		return drained
	}
	return namespace.Release(cleanup)
}

func recoverInherited(ctx context.Context, core *Core) error {
	config := core.Config
	if err := VerifyInherited(config.Data); err != nil {
		return err
	}
	if err := ReleaseProbes(ctx, config.Data); err != nil {
		return err
	}
	return FenceAndSave(ctx, core)
}
