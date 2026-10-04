//go:build !windows

package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/commandproto"
)

type devProcess struct {
	command *exec.Cmd
	done    chan struct{}
	err     error
}

func startDevProcess(ctx context.Context, command *exec.Cmd) (*devProcess, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := command.Start(); err != nil {
		return nil, err
	}
	process := &devProcess{command: command, done: make(chan struct{})}
	go func() {
		process.err = command.Wait()
		close(process.done)
	}()
	return process, nil
}

func (p *devProcess) stop(ctx context.Context, terminate bool, patience time.Duration) error {
	select {
	case <-p.done:
		return nil
	default:
	}
	if terminate {
		if err := syscall.Kill(-p.command.Process.Pid, syscall.SIGTERM); err != nil && !errors.Is(err, syscall.ESRCH) {
			return errors.Join(err, p.kill(context.WithoutCancel(ctx)))
		}
	}
	timer := time.NewTimer(patience)
	defer timer.Stop()
	select {
	case <-p.done:
		return nil
	case <-timer.C:
		return p.kill(context.WithoutCancel(ctx))
	case <-ctx.Done():
		return p.kill(context.WithoutCancel(ctx))
	}
}

func (p *devProcess) kill(_ context.Context) error {
	err := syscall.Kill(-p.command.Process.Pid, syscall.SIGKILL)
	if errors.Is(err, syscall.ESRCH) {
		err = nil
	}
	<-p.done
	return err
}

func (a *application) serveDev(ctx context.Context, o devOptions, root, programs string) (err error) {
	echo, err := startEcho(ctx)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, echo.close(context.WithoutCancel(ctx))) }()
	managerCmd := exec.Command(
		filepath.Join(programs, "scripted-machines"),
		"--artifacts",
		filepath.Join(a.Root, ".cache/dev-artifacts"),
	)
	managerCmd.Dir = a.Root
	managerCmd.Env = append(devEnvironment(), "DEMI_TEST_PROGRAMS="+programs)
	managerCmd.Stderr = a.Err
	input, err := managerCmd.StdinPipe()
	if err != nil {
		return err
	}
	defer func() { _ = input.Close() }() // Idempotent shutdown after the manager's input is closed below.
	output, err := managerCmd.StdoutPipe()
	if err != nil {
		return err
	}
	manager, err := startDevProcess(ctx, managerCmd)
	if err != nil {
		_ = output.Close()
		return err
	}
	defer func() {
		_ = input.Close()
		err = errors.Join(err, manager.stop(context.Background(), false, 5*time.Second))
	}()
	// The pipe reader belongs to the session; closing it wakes both its initial
	// line read and its forwarding copy, and the session joins it on every exit.
	socket := make(chan string, 1)
	forwarded := a.forwardDevOutput(output, socket)
	defer func() {
		_ = output.Close() // Closing the pipe wakes the owned reader during cleanup.
		<-forwarded
	}()
	timer := time.NewTimer(30 * time.Second)
	defer timer.Stop()
	address, err := devManagerAddress(ctx, socket, manager, timer.C)
	if err != nil {
		return err
	}
	if err := a.publishDevCommands(ctx, root, programs); err != nil {
		return err
	}
	return a.serveDevBackend(ctx, o, root, programs, address, echo, manager)
}

func (a *application) publishDevCommands(ctx context.Context, root, programs string) error {
	target, err := commandproto.HostTarget()
	if err != nil {
		return err
	}
	chrome, err := a.chromeRelease()
	if err != nil {
		return err
	}
	for _, name := range commandPrograms {
		if err := a.publishNative(
			ctx,
			packageOptions{Package: name, Output: filepath.Join(root, "releases", name)},
			map[string]string{string(target): filepath.Join(programs, name)},
			chrome,
		); err != nil {
			return err
		}
	}

	return nil
}

func (a *application) serveDevBackend(
	ctx context.Context,
	o devOptions,
	root, programs, address string,
	echo *echoServer,
	manager *devProcess,
) (err error) {
	native, err := writeDevConfig(ctx, root)
	if err != nil {
		return err
	}
	origin := fmt.Sprintf("http://127.0.0.1:%d", o.Port)
	backendCmd := exec.Command(filepath.Join(programs, "demi-backend"))
	backendCmd.Dir = a.Root
	backendCmd.Stdout = a.Out
	backendCmd.Stderr = a.Err
	backendCmd.Env = append(
		devEnvironment(),
		"DEMI_BACKEND_DATA="+filepath.Join(root, "backend"),
		fmt.Sprintf("DEMI_BACKEND_PORT=%d", o.Port),
		"DEMI_INSTANCE_MODE=isolated",
		"DEMI_BACKEND_PUBLIC_URL="+origin,
		"DEMI_MACHINE_MANAGER_SOCKET="+strings.TrimRight(address, "\r\n"),
		"DEMI_NATIVE_CONFIG="+native,
	)
	backend, err := startDevProcess(ctx, backendCmd)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, backend.stop(context.Background(), true, 10*time.Second)) }()
	client, err := devClient()
	if err != nil {
		return err
	}
	defer client.CloseIdleConnections()
	if err := a.seedDevBackend(ctx, origin, root, echo, backend, client, o.Provider); err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return nil
	case <-backend.done:
		return processExit("the backend exited", backend.err)
	case <-manager.done:
		return processExit("the machine manager exited", manager.err)
	}
}

func (a *application) seedDevBackend(
	ctx context.Context,
	origin, root string,
	echo *echoServer,
	backend *devProcess,
	client *http.Client,
	p *devProvider,
) error {
	if err := answeringDev(ctx, client, origin, backend); err != nil {
		return err
	}
	provider, err := seedDev(ctx, client, origin, echo.url)
	if err != nil {
		return err
	}
	var modelSummary string
	if p != nil {
		id, err := seedDevProvider(ctx, client, origin, *p)
		if err != nil {
			return err
		}
		modelSummary = fmt.Sprintf(
			"  Model:   %s of the provider entry %s (Development, %s)\n",
			p.Model, id, p.BaseURL,
		)
	}
	if _, err := fmt.Fprintf(
		a.Out,
		"\nThe development backend serves at %s\n  Account: %s, password %s\n  "+
			"Model:   echo of the provider entry %s, which answers \"Echo: <your "+
			"message>\"\n%s  Data:    %s\nStart the page in another terminal:\n  "+
			"DEMI_BACKEND_URL=%s DEMI_DEV_EMAIL=%s DEMI_DEV_PASSWORD=%s bun run "+
			"web:dev\nCtrl-C stops the backend.\n",
		origin,
		devEmail,
		devPassword,
		provider,
		modelSummary,
		root,
		origin,
		devEmail,
		devPassword,
	); err != nil {
		return err
	}

	return nil
}

func devManagerAddress(
	ctx context.Context,
	socket <-chan string,
	manager *devProcess,
	timeout <-chan time.Time,
) (string, error) {
	var address string
	select {
	case address = <-socket:
	case <-manager.done:
		return "", processExit("the machine manager exited before it printed its socket", manager.err)
	case <-ctx.Done():
		return "", ctx.Err()
	case <-timeout:
		return "", errors.New("the machine manager printed no socket within 30 s")
	}

	return address, nil
}

// forwardDevOutput returns the join channel; serveDev owns the pipe and closes it before joining.
func (a *application) forwardDevOutput(output io.Reader, socket chan<- string) <-chan struct{} {
	forwarded := make(chan struct{})
	go func() {
		defer close(forwarded)
		reader := bufio.NewReader(output)
		line, readErr := reader.ReadString('\n')
		if readErr == nil {
			socket <- line
		}
		_, _ = io.Copy(a.Out, reader)
	}()

	return forwarded
}
