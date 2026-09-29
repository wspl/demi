//go:build linux

package machines

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"os"
	"os/signal"
	"syscall"

	"github.com/coreos/go-systemd/v22/daemon"

	"github.com/wspl/demi/go/internal/envflag"
	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/machinesproto"
)

// deaths is how many deaths wait for the server's fan-out, which takes them at
// once.
const deaths = 64

// Main is the demi-machines executable: the one place the manager is assembled
// (docs/cloud/managed-hosts.md § Startup and recovery). It returns the exit
// status.
func Main() int {
	cfg, err := config.Parse(os.Args[1:], os.Environ(), os.Stdout)
	if err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return 0
		}
		fmt.Fprintf(os.Stderr, "demi-machines: %v\n", err)
		if envflag.IsUsage(err) {
			return 2
		}
		return 1
	}
	// The journal records the service's standard error and the time; the
	// messages name their part of the manager themselves.
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{
		Level: slog.LevelDebug,
		ReplaceAttr: func(groups []string, attr slog.Attr) slog.Attr {
			if len(groups) == 0 && attr.Key == slog.TimeKey {
				return slog.Attr{}
			}
			return attr
		},
	})))
	if err := run(context.Background(), cfg); err != nil {
		slog.Error("demi-machines: " + Chain(err))
		return 1
	}
	return 0
}

func run(ctx context.Context, cfg *config.Config) error {
	if err := requireRoot(); err != nil {
		return err
	}
	if err := requirePrivateNamespace(); err != nil {
		return err
	}
	programs, err := tools.Resolve(cfg.Runsc)
	if err != nil {
		return err
	}
	core := NewCore(cfg, programs)
	if err := requireRunsc(ctx, core); err != nil {
		return err
	}
	runtime := cfg.Runtime()
	if err := storage.CreatePrivate(cfg.Data); err != nil {
		return err
	}
	if err := storage.CreatePrivate(runtime); err != nil {
		return err
	}
	if cfg.Mode == config.RecoverNamespace {
		// The manager that started this process holds the locks and waits for
		// it, inside the namespace it recovers.
		if err := VerifyInherited(cfg.Data); err != nil {
			return err
		}
		if err := releaseProbes(cfg.Data); err != nil {
			return err
		}
		return fenceAndSave(ctx, core)
	}
	lock, err := AcquireLocks(cfg.Data, runtime)
	if err != nil {
		return err
	}
	defer lock.Close()
	namespace := NewSavedNamespace(runtime, cfg.Data)
	if err := namespace.Recover(ctx, lock); err != nil {
		return err
	}
	if err := releaseProbes(cfg.Data); err != nil {
		return err
	}
	if cfg.Limits != nil {
		if err := core.Cgroups.Prepare(); err != nil {
			return err
		}
	} else {
		slog.Warn("demi-machines: DEMI_MANAGED_LIMITS=off: Clouds run without CPU, memory or PID limits")
	}
	if err := requireOneFilesystem(cfg.Working(), cfg.Images()); err != nil {
		return err
	}
	if err := fenceAndSave(ctx, core); err != nil {
		return err
	}
	if cfg.Mode == config.Recover {
		return nil
	}
	if err := core.Network.Prepare(ctx); err != nil {
		return err
	}
	base, err := storage.ImportBase(ctx, programs, cfg.Image, core.Store.Bases())
	if err != nil {
		return err
	}
	if err := namespace.Pin(); err != nil {
		return err
	}
	if err := probeStorage(ctx, core); err != nil {
		return err
	}
	socket, err := Bind(cfg.Socket)
	if err != nil {
		return err
	}
	return serve(ctx, core, base, socket, namespace)
}

// serve serves the backend until SIGTERM or SIGINT, then drains the devices. The
// state is ready to serve: recovered, the network policy installed and the base
// imported.
func serve(ctx context.Context, core *Core, base machinesproto.BaseVersion, socket *Socket, namespace *SavedNamespace) error {
	cfg := core.Config
	died := make(chan machinesproto.DeviceID, deaths)
	manager := NewManager(ctx, core, base, died)
	stopping, stop := signal.NotifyContext(ctx, syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	// Readiness for systemd's Type=notify; without a notify socket this does
	// nothing.
	if _, err := daemon.SdNotify(false, daemon.SdNotifyReady); err != nil {
		return err
	}
	limits := "off"
	if cfg.Limits != nil {
		limits = "on"
	}
	slog.Info(fmt.Sprintf("demi-machines: gVisor/systrap ready at %s, resource limits %s", cfg.Socket, limits))
	inFlight := Serve(stopping, socket, manager, died)
	drained := manager.Close(ctx)
	inFlight.Wait()
	if drained != nil {
		return drained
	}
	// Only a successful drain releases the handle; otherwise the service's
	// stop-post recovery works through it.
	return namespace.Release()
}
