//go:build linux

package machinemanager_test

import (
	"bufio"
	"bytes"
	"errors"
	"flag"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/internal/machinemanager"
	"github.com/wspl/demi/internal/machinemanager/machinemanagertest"
	"github.com/wspl/demi/internal/machinemanager/sandbox"
	"github.com/wspl/demi/internal/machineproto"
	"github.com/wspl/demi/internal/programtest"
	"golang.org/x/sys/unix"
)

var processTests = flag.Bool(
	"machines-process",
	false,
	"run manager process scenarios; requires root, namespace and filesystem tools",
)

type testHost struct {
	command *exec.Cmd
	cpu     string
}

func startHost(t *testing.T, controllers bool) *testHost {
	t.Helper()
	if !*processTests {
		t.Skip("opt in with -machines-process on privileged Linux with namespace and filesystem tools")
	}
	var allowed unix.CPUSet
	if err := unix.SchedGetaffinity(0, &allowed); err != nil {
		t.Fatal(err)
	}
	cpu := ""
	for i := 0; i < 1024; i++ {
		if allowed.IsSet(i) {
			cpu = strconv.Itoa(i)
			break
		}
	}
	if cpu == "" {
		t.Fatal("no allowed CPU")
	}
	mounts := "mount -t tmpfs -o ro tmpfs /sys/fs/cgroup"
	if controllers {
		mounts = "mount -t tmpfs tmpfs /sys/fs/cgroup; echo 'cpu memory pids' > /sys/fs/cgroup/cgroup.controllers"
	}
	script := "set -e\nmount --make-rshared /\n" +
		"mount -t tmpfs -o mode=0755 tmpfs /run\n" + mounts + "\necho ready\nexec sleep infinity\n"
	command := exec.CommandContext(
		t.Context(),
		"taskset",
		"--cpu-list",
		cpu,
		"unshare",
		"--mount",
		"--net",
		"--pid",
		"--fork",
		"--mount-proc",
		"--kill-child",
		"--",
		"sh",
		"-c",
		script,
	)
	var diagnostic bytes.Buffer
	command.Stderr = &diagnostic
	stdout, err := command.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err = command.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = command.Process.Kill()
		_ = command.Wait()
		_ = stdout.Close()
	})
	ready, err := bufio.NewReader(stdout).ReadString('\n')
	if err != nil || ready != "ready\n" {
		t.Fatalf("host readiness: %q %v", ready, err)
	}
	return &testHost{command: command, cpu: cpu}
}

func (h *testHost) runtimeFile(name string) string {
	return fmt.Sprintf("/proc/%d/root%s/%s", h.command.Process.Pid, machinemanager.RuntimeDirectory, name)
}

type processSettings struct {
	env    []string
	notify *net.UnixConn
	data   string
}

func settings(t *testing.T) *processSettings {
	t.Helper()
	architecture, ok := machineproto.HostArchitecture()
	if !ok {
		t.Fatal("unsupported architecture")
	}
	image := machinemanagertest.NewCloudImage(t, machinemanagertest.Entries(), architecture)
	directory := t.TempDir()
	runsc := filepath.Join(directory, "runsc")
	script := "#!/bin/sh\n" +
		"[ \"$1\" = --version ] || { echo \"runsc $*\" >&2; exit 1; }\necho 'runsc version " + sandbox.PinnedRelease().
		Version() +
		"'\n"
	if err := os.WriteFile(runsc, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(directory, "notify")
	notify, err := net.ListenUnixgram("unixgram", &net.UnixAddr{Name: path, Net: "unixgram"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = notify.Close()
	})
	raw, err := notify.SyscallConn()
	if err != nil {
		t.Fatal(err)
	}
	var optionErr error
	if err = raw.Control(
		func(fd uintptr) {
			optionErr = unix.SetsockoptInt(int(fd), unix.SOL_SOCKET, unix.SO_PASSCRED, 1)
		},
	); err != nil {
		t.Fatal(err)
	}
	if optionErr != nil {
		t.Fatal(optionErr)
	}
	data := filepath.Join(directory, "data")
	return &processSettings{
		env: []string{
			"PATH=/usr/sbin:/usr/bin:/sbin:/bin",
			"NOTIFY_SOCKET=" + path,
			"DEMI_MACHINE_MANAGER_SOCKET=" + filepath.Join(directory, "machines.sock"),
			"DEMI_MACHINE_MANAGER_DATA=" + data,
			"DEMI_MANAGED_RUNSC=" + runsc,
			"DEMI_MANAGED_IMAGE=" + image.Directory,
			"DEMI_MANAGED_BACKEND_URL=http://203.0.113.10:3271",
			"DEMI_MANAGED_DNS=1.1.1.1",
			"DEMI_MANAGED_SLOTS=4",
		},
		notify: notify,
		data:   data,
	}
}

type managerProcess struct {
	command *exec.Cmd
	output  bytes.Buffer
	done    chan struct{}
	err     error
	pid     int
}

func (h *testHost) manager(t *testing.T, s *processSettings, args ...string) *managerProcess {
	t.Helper()
	program, err := programtest.Path(t.Context(), "demi-machine-manager")
	if err != nil {
		t.Fatal(err)
	}
	pid := h.command.Process.Pid
	flags := []string{
		fmt.Sprintf("--mount=/proc/%d/ns/mnt", pid),
		fmt.Sprintf("--net=/proc/%d/ns/net", pid),
		fmt.Sprintf("--pid=/proc/%d/ns/pid_for_children", pid),
		"--",
		"taskset",
		"--cpu-list",
		h.cpu,
		"unshare",
		"--mount",
		"--propagation",
		"slave",
		"--",
		program,
	}
	p := &managerProcess{done: make(chan struct{})}
	p.command = exec.CommandContext(t.Context(), "nsenter", append(flags, args...)...)
	p.command.Env = s.env
	p.command.Stderr = &p.output
	if err = p.command.Start(); err != nil {
		t.Fatal(err)
	}
	p.pid = p.command.Process.Pid
	go func() {
		p.err = p.command.Wait()
		close(p.done)
	}()
	t.Cleanup(func() {
		select {
		case <-p.done:
		default:
			_ = syscall.Kill(p.pid, syscall.SIGKILL)
			_ = p.command.Process.Kill()
			<-p.done
		}
	})
	return p
}

func (p *managerProcess) ready(t *testing.T, s *processSettings) {
	t.Helper()
	result := make(chan error, 1)
	var reader sync.WaitGroup
	reader.Go(func() {
		buffer := make([]byte, 256)
		control := make([]byte, unix.CmsgSpace(unix.SizeofUcred))
		n, oobn, _, _, err := s.notify.ReadMsgUnix(buffer, control)
		if err == nil {
			messages, e := unix.ParseSocketControlMessage(control[:oobn])
			if e != nil {
				err = e
			} else {
				for _, message := range messages {
					credentials, e := unix.ParseUnixCredentials(&message)
					if e != nil {
						err = e
						break
					}
					p.pid = int(credentials.Pid)
				}
			}
		}
		if err == nil && !strings.Contains(string(buffer[:n]), "READY=1") {
			err = fmt.Errorf("unexpected readiness: %s", buffer[:n])
		}
		result <- err
	})
	select {
	case err := <-result:
		reader.Wait()
		if err != nil {
			t.Fatal(err)
		}
	case <-p.done:
		_ = s.notify.SetReadDeadline(time.Now())
		reader.Wait()
		t.Fatalf("manager exited before readiness: %v: %s", p.err, p.output.String())
	}
}

func (p *managerProcess) stop(t *testing.T, signal syscall.Signal) {
	t.Helper()
	if err := syscall.Kill(p.pid, signal); err != nil {
		t.Fatal(err)
	}
	<-p.done
}

// Cost: two complete manager starts plus stop-post recovery; no real runsc sandbox.
func TestNextStartRecoversKilledManagerNamespace(t *testing.T) {
	host := startHost(t, true)
	s := settings(t)
	first := host.manager(t, s)
	first.ready(t, s)
	pinned, err := os.Stat(host.runtimeFile("mount-namespace"))
	if err != nil {
		t.Fatal(err)
	}
	own, err := os.Stat(fmt.Sprintf("/proc/%d/ns/mnt", first.pid))
	if err != nil || !os.SameFile(pinned, own) {
		t.Fatalf("pin: %v", err)
	}
	owner, err := os.ReadFile(host.runtimeFile("mount-namespace-owner.json"))
	if err != nil {
		t.Fatal(err)
	}
	if string(owner) != `{"dataDir":`+strconv.Quote(s.data)+`}` {
		t.Fatalf("namespace owner: %s", owner)
	}
	first.stop(t, syscall.SIGKILL)
	kept, err := os.Stat(host.runtimeFile("mount-namespace"))
	if err != nil || !os.SameFile(kept, pinned) {
		t.Fatal("crash lost namespace handle")
	}
	second := host.manager(t, s)
	second.ready(t, s)
	replacement, err := os.Stat(host.runtimeFile("mount-namespace"))
	if err != nil || os.SameFile(replacement, pinned) {
		t.Fatal("recovery did not replace namespace")
	}
	secondOwn, err := os.Stat(fmt.Sprintf("/proc/%d/ns/mnt", second.pid))
	if err != nil || !os.SameFile(replacement, secondOwn) {
		t.Fatalf("replacement pin: %v", err)
	}
	second.stop(t, syscall.SIGKILL)
	recovery := host.manager(t, s, "--recover")
	<-recovery.done
	if recovery.err != nil {
		t.Fatalf("recovery: %v: %s", recovery.err, recovery.output.String())
	}
	for _, name := range []string{"mount-namespace", "mount-namespace-owner.json"} {
		if _, err = os.Stat(host.runtimeFile(name)); !os.IsNotExist(err) {
			t.Fatalf("retained %s: %v", name, err)
		}
	}
}

// Cost: startup rejection before storage import, normally <1 s.
func TestLimitsOnNamesMissingControllers(t *testing.T) {
	host := startHost(t, false)
	s := settings(t)
	p := host.manager(t, s)
	<-p.done
	if p.err == nil {
		t.Fatal("started without controllers")
	}

	raw, err := s.notify.SyscallConn()
	if err != nil {
		t.Fatal(err)
	}
	var readErr error
	if err := raw.Control(
		func(fd uintptr) {
			_, _, readErr = unix.Recvfrom(int(fd), make([]byte, 4096), unix.MSG_DONTWAIT)
		},
	); err != nil {
		t.Fatal(err)
	}
	if !errors.Is(readErr, unix.EAGAIN) {
		t.Fatalf("startup sent readiness before failing: %v", readErr)
	}
	text := p.output.String()
	if !strings.Contains(text, "missing: cpu, memory, pids.") || !strings.Contains(text, "DEMI_MANAGED_LIMITS=off") {
		t.Fatal(text)
	}
}

// Cost: one complete startup and graceful drain without cgroups.
func TestLimitsOffStartsWithoutCgroups(t *testing.T) {
	host := startHost(t, false)
	s := settings(t)
	s.env = append(s.env, "DEMI_MANAGED_LIMITS=off")
	p := host.manager(t, s)
	p.ready(t, s)
	var socket string
	for _, entry := range s.env {
		if value, ok := strings.CutPrefix(entry, "DEMI_MACHINE_MANAGER_SOCKET="); ok {
			socket = value
		}
	}
	client := (&serverFixture{path: socket}).connect(t)
	client.send(t, "{\"id\":\"reconcile\",\"op\":\"reconcile\",\"params\":{}}\n")
	if reply, ok := client.receive(t).(*machineproto.OK); !ok || reply.ID != "reconcile" {
		t.Fatalf("reconcile: %+v", reply)
	}
	p.stop(t, syscall.SIGTERM)
	if p.err != nil {
		t.Fatalf("stop: %v: %s", p.err, p.output.String())
	}
	text := p.output.String()
	if !strings.Contains(text, "DEMI_MANAGED_LIMITS=off: Clouds run without CPU, memory or PID limits") ||
		!strings.Contains(text, "resource limits off") {
		t.Fatal(text)
	}
}
