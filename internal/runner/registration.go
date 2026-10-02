package runner

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"time"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/host"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

type registrationOptions struct {
	backend                                        runnerwire.BackendURL
	directory, artifacts, jobRoot, executable, cwd string
	env                                            map[string]string
	runner                                         runnerwire.RunnerInfo
	token                                          *runnerwire.DeviceToken
	volumes                                        []host.ManagedVolume
	shell                                          process.JobShell
	log                                            *hostLog
}

// registration holds resources that survive a backend reconnect.
type registration struct {
	options    registrationOptions
	state      runnerState
	token      atomic.Pointer[runnerwire.DeviceToken]
	management *management
	services   *cmdpkgs.ServiceHandle
	installs   *cmdpkgs.InstallsReceiver
	dispatcher *jobs.Dispatcher
	contexts   *jobs.Contexts
	paths      jobs.ContextPaths
	pipes      *process.PipeClient
	reserved   map[string]struct{}
	endpoint   string
}

func runRegistration(ctx context.Context, options registrationOptions) (err error) {
	if err := openInstallation(ctx, options.directory); err != nil {
		return err
	}
	lease, err := tryInstallationLock(options.directory)
	if err != nil {
		return err
	}
	if lease == nil {
		return errors.New("runner already active for this installation")
	}
	defer func() { err = errors.Join(err, lease.close()) }()
	state := runnerState{root: options.directory}
	saved, err := state.config()
	if err != nil {
		return err
	}
	if saved != nil && instanceID(saved.BackendURL) != instanceID(options.backend) {
		return errors.New("installation is registered to another backend")
	}
	token := options.token
	if token == nil {
		token, err = state.token()
		if err != nil {
			return err
		}
	}
	if options.runner.Managed != nil && *options.runner.Managed && token == nil {
		return errors.New("managed runner requires a device token")
	}
	config := runnerConfig{BackendURL: options.backend}
	if saved != nil {
		config.DeviceID = saved.DeviceID
	}
	if err := state.writeConfig(ctx, config); err != nil {
		return err
	}
	slog.Info("runner " + options.runner.Version + " started")
	image := ""
	if runtime.GOOS != "windows" {
		image = runnerwire.ArtifactsPath
	}
	// Shutdown is explicit: cancellation must not kill services before connection work joins.
	lifetime, cancel := context.WithCancel(context.Background())
	defer cancel()
	registry, err := cmdpkgs.NewServiceRegistry(lifetime, options.artifacts, image, options.cwd, options.env)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, registry.Close(context.Background())) }()
	paths, err := jobs.NewContextPaths(ctx, filepath.Join(state.root, "commands"), options.executable)
	if err != nil {
		return err
	}
	id, err := uuid.NewRandom()
	if err != nil {
		return err
	}
	management := newManagement(strings.ReplaceAll(id.String(), "-", ""), options.runner.Version)
	r := &registration{options: options, state: state, management: management, services: registry.Handle(), installs: registry.Installs(), contexts: &jobs.Contexts{}, paths: paths}
	r.token.Store(token)
	r.pipes, err = process.NewPipeClient(options.backend, func() (runnerwire.DeviceToken, bool) {
		token := r.token.Load()
		if token == nil {
			return "", false
		}
		return *token, true
	})
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, r.pipes.Close()) }()
	r.dispatcher = &jobs.Dispatcher{Contexts: r.contexts, Services: r.services, Pipes: r.pipes}
	server, err := jobs.StartServer(lifetime, &endpoint{dispatcher: r.dispatcher, management: management})
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, server.Close(context.Background())) }()
	r.endpoint = server.Endpoint()
	r.reserved = options.shell.BuiltinNames()
	r.reserved[program] = struct{}{}
	if err := lease.publish(ctx, state, activeRunner{Endpoint: r.endpoint, Secret: management.secret, Release: options.runner.Version}); err != nil {
		return err
	}
	err = r.reconnect(ctx)
	if management.snapshot().Draining && ctx.Err() == nil {
		err = errors.Join(err, server.WaitIdle(ctx))
	}
	// Service registry must end before the local server and installation lock.
	err = errors.Join(err, registry.Close(context.Background()))
	slog.Info("runner stopped")
	return err
}

func (r *registration) reconnect(ctx context.Context) error {
	delay := 250 * time.Millisecond
	failure := ""
	for {
		if ctx.Err() != nil || r.management.snapshot().Draining {
			return nil
		}
		r.management.setPhase(connecting)
		if failure == "" {
			slog.Info("connecting to " + r.options.backend.String())
		}
		opening, cancel := context.WithCancel(ctx)
		joined := make(chan struct{})
		go func() {
			defer close(joined)
			select {
			case <-r.management.draining:
				cancel()
			case <-opening.Done():
			}
		}()
		transport, err := connect(opening, r.options.backend)
		cancel()
		<-joined
		end := disconnected
		if err == nil {
			end, err = r.serve(ctx, transport)
		}
		if err == nil {
			switch end {
			case refused:
				return errors.New("backend rejected runner registration")
			case stopped:
				return nil
			case disconnected:
				slog.Warn("backend connection lost")
				failure = ""
				delay = 250 * time.Millisecond
			}
		} else {
			text := "connection ended: " + err.Error()
			if text == failure {
				_, _ = fmt.Fprintln(os.Stderr, "demi-runner: "+text)
			} else {
				slog.Warn(text)
			}
			failure = text
		}
		timer := time.NewTimer(delay)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil
		case <-r.management.draining:
			timer.Stop()
			return nil
		case <-timer.C:
		}
		delay = min(delay*2, 10*time.Second)
	}
}
