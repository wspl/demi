//go:build linux

package machines_test

import (
	"bufio"
	"bytes"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io/fs"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"golang.org/x/sys/unix"

	"github.com/wspl/demi/go/machines/internal/imagetest"
	"github.com/wspl/demi/go/machines/internal/roottest"
	"github.com/wspl/demi/go/machines/internal/sandbox"
	"github.com/wspl/demi/go/machinesproto"
)

// The manager as a process (docs/cloud/managed-hosts.md § Startup and recovery,
// § Resource limits): the built executable, started the way its unit starts it, in
// a stand-in execution host. The host is PID 1 of new PID, mount and network
// namespaces with its own /proc, /run and cgroup root, so the managers' namespace
// handle, locks, firewall and cgroups exist only there, and its end kills
// everything inside.

const (
	// handle and owner are the namespace handle and its owner record in the
	// runtime directory (namespace.go).
	handle = "mount-namespace"
	owner  = "mount-namespace-owner.json"
	// runtimeDirectory is where the manager keeps its runtime files.
	runtimeDirectory = "/run/demi-machines"
	// readyDeadline is how long a manager may take to start.
	readyDeadline = 60 * time.Second
)

var (
	buildOnce sync.Once
	built     string
	buildErr  error
)

// manager builds the executable once per test binary.
func manager(t *testing.T) string {
	t.Helper()
	// A test of another executable, such as one run under a tracer, names it.
	if executable := os.Getenv("DEMI_TEST_MACHINES"); executable != "" {
		return executable
	}
	buildOnce.Do(func() {
		directory, err := os.MkdirTemp("", "demi-machines-test-")
		if err != nil {
			buildErr = err
			return
		}
		built = filepath.Join(directory, "demi-machines")
		output, err := exec.Command("go", "build", "-o", built, "github.com/wspl/demi/go/cmd/demi-machines").CombinedOutput()
		if err != nil {
			buildErr = fmt.Errorf("go build: %v\n%s", err, output)
		}
	})
	if buildErr != nil {
		t.Fatal(buildErr)
	}
	return built
}

// removeBuiltManager removes the executable the process tests built, and the
// directory it was built in.
func removeBuiltManager() {
	if built != "" {
		_ = os.RemoveAll(filepath.Dir(built)) // A leftover in the temporary directory is harmless.
	}
}

// init is the host's init: the host's mounts are shared, as systemd's are, and its
// /run is its own. So is its cgroup root, which the host's cgroups argument
// prepares.
const initScript = `set -e
mount --make-rshared /
mount -t tmpfs -o mode=0755 tmpfs /run
eval "$1"
echo ready
exec sleep infinity`

// A cgroups is the stand-in host's cgroup root.
type cgroups string

const (
	// offered is a root that offers the controllers the limits need; no sandbox
	// runs, so a stand-in that takes the manager's writes does.
	offered cgroups = "mount -t tmpfs tmpfs /sys/fs/cgroup && echo 'cpu memory pids' > /sys/fs/cgroup/cgroup.controllers"
	// absent is no cgroup hierarchy at all, and nothing can be created there.
	absent cgroups = "mount -t tmpfs -o ro tmpfs /sys/fs/cgroup"
)

// namespaceCPU is the CPU the stand-in host's mount namespace and every manager's
// are made on. Linux 6.18 numbers namespaces from per-CPU batches (gen_cookie_next
// in kernel/nstree.c), so a namespace made after the host's, but on another CPU,
// can get the lower number, and mnt_ns_loop in fs/namespace.c then refuses to bind
// it into the host's namespace, which fails the manager's pin at random.
// Namespaces made on one CPU are numbered in the order they are made. A real
// host's namespace is the initial one, numbered before all others
// (docs/cloud/managed-hosts.md § Startup and recovery).
func namespaceCPU(t *testing.T) string {
	t.Helper()
	var allowed unix.CPUSet
	if err := unix.SchedGetaffinity(0, &allowed); err != nil {
		t.Fatal(err)
	}
	for cpu := 0; cpu < len(allowed)*64; cpu++ {
		if allowed[cpu/64]&(1<<(cpu%64)) != 0 {
			return strconv.Itoa(cpu)
		}
	}
	t.Fatal("the test may run on no CPU")
	return ""
}

// A settings is the manager's settings (docs/cloud/setup.md § Configuration) for a
// state directory, an image release and a runsc that only reports the pinned
// version.
type settings struct {
	// notify is where the manager reports readiness, as systemd's NOTIFY_SOCKET.
	notify    string
	data      string
	variables []string
}

func newSettings(t *testing.T, directory, image string) *settings {
	t.Helper()
	runsc := filepath.Join(directory, "runsc")
	script := fmt.Sprintf("#!/bin/sh\n[ \"$1\" = --version ] || { echo \"runsc $*\" >&2; exit 1; }\necho 'runsc version %s'\n", sandbox.PinnedRelease().Version())
	if err := os.WriteFile(runsc, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	notify := filepath.Join(directory, "notify")
	data := filepath.Join(directory, "data")
	return &settings{
		notify: notify,
		data:   data,
		variables: []string{
			"PATH=/usr/sbin:/usr/bin:/sbin:/bin",
			"NOTIFY_SOCKET=" + notify,
			"DEMI_MACHINES_SOCKET=" + filepath.Join(directory, "machines.sock"),
			"DEMI_MACHINES_DATA=" + data,
			"DEMI_MANAGED_RUNSC=" + runsc,
			"DEMI_MANAGED_IMAGE=" + image,
			"DEMI_MANAGED_BACKEND_URL=http://203.0.113.10:3271",
			"DEMI_MANAGED_DNS=1.1.1.1",
			"DEMI_MANAGED_SLOTS=4",
		},
	}
}

// A host is the stand-in execution host: unshare's child is its init, PID 1.
type host struct {
	t       *testing.T
	unshare *exec.Cmd
}

func startHost(t *testing.T, root cgroups) *host {
	t.Helper()
	unshare := exec.Command("taskset", "--cpu-list", namespaceCPU(t), "unshare", "--mount", "--net", "--pid", "--fork", "--mount-proc", "--kill-child",
		"--", "sh", "-c", initScript, "init", string(root))
	stdout, err := unshare.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := unshare.Start(); err != nil {
		t.Fatal(err)
	}
	h := &host{t: t, unshare: unshare}
	t.Cleanup(func() {
		// unshare's end kills the init (--kill-child), and with it every process of
		// the host.
		_ = unshare.Process.Kill()
		_ = unshare.Wait()
	})
	line, err := bufio.NewReader(stdout).ReadString('\n')
	if err != nil || line != "ready\n" {
		t.Fatalf("the host did not start: %q, %v", line, err)
	}
	return h
}

// runtimeFile returns a file of the runtime directory in the host's mount
// namespace, which unshare shares with its init.
func (h *host) runtimeFile(name string) string {
	return fmt.Sprintf("/proc/%d/root%s/%s", h.unshare.Process.Pid, runtimeDirectory, name)
}

// A namespaceID is a mount namespace's identity: its device and inode.
type namespaceID [2]uint64

func identity(t *testing.T, path string) *namespaceID {
	t.Helper()
	info, err := os.Stat(path)
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		t.Fatal(err)
	}
	stat := info.Sys().(*syscall.Stat_t)
	return &namespaceID{uint64(stat.Dev), stat.Ino}
}

// handle returns the namespace the host's handle holds; nil without a handle.
func (h *host) handle() *namespaceID {
	return identity(h.t, h.runtimeFile(handle))
}

// A running is a manager process started by the host.
type running struct {
	t       *testing.T
	nsenter *exec.Cmd
	stderr  *lockedBuffer
	done    chan error
	// found is the manager's process id, once it has been looked up.
	found int
}

// manager starts the manager as its unit does, with PrivateMounts=yes: in the
// host's PID and network namespaces and in a mount namespace of its own that
// receives the host's mounts.
func (h *host) manager(s *settings, args ...string) *running {
	h.t.Helper()
	init := h.unshare.Process.Pid
	cpu := namespaceCPU(h.t)
	// On the host's CPU, so this namespace gets the higher number (namespaceCPU).
	command := []string{
		fmt.Sprintf("--mount=/proc/%d/ns/mnt", init),
		fmt.Sprintf("--net=/proc/%d/ns/net", init),
		fmt.Sprintf("--pid=/proc/%d/ns/pid_for_children", init),
		"--", "taskset", "--cpu-list", cpu,
		"unshare", "--mount", "--propagation", "slave", "--", manager(h.t),
	}
	nsenter := exec.Command("nsenter", append(command, args...)...)
	nsenter.Env = s.variables
	stderr := &lockedBuffer{}
	nsenter.Stderr = stderr
	if err := nsenter.Start(); err != nil {
		h.t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { done <- nsenter.Wait() }()
	return &running{t: h.t, nsenter: nsenter, stderr: stderr, done: done}
}

// pid returns the manager's process id: nsenter forks the child that enters the
// PID namespace, and that child becomes the manager. A manager that failed its
// start may be gone before it can be looked up, so only a test that needs the
// process asks.
func (r *running) pid() int {
	r.t.Helper()
	if r.found != 0 {
		return r.found
	}
	children := fmt.Sprintf("/proc/%d/task/%[1]d/children", r.nsenter.Process.Pid)
	deadline := time.Now().Add(readyDeadline)
	for {
		listed, _ := os.ReadFile(children)
		if fields := strings.Fields(string(listed)); len(fields) > 0 {
			r.found, _ = strconv.Atoi(fields[0])
			return r.found
		}
		select {
		case status := <-r.done:
			r.done <- status
			r.t.Fatalf("nsenter exited (%v) with no manager: %s", status, r.errors())
		case <-time.After(10 * time.Millisecond):
		}
		if time.Now().After(deadline) {
			r.t.Fatal("nsenter started no manager")
		}
	}
}

// lockedBuffer is a buffer that its writer and its reader may use together.
type lockedBuffer struct {
	mu     sync.Mutex
	buffer bytes.Buffer
}

func (b *lockedBuffer) Write(data []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buffer.Write(data)
}

func (b *lockedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buffer.String()
}

// errors returns what the manager wrote to its standard error so far.
func (r *running) errors() string {
	return r.stderr.String()
}

// A start is how a start ended: in readiness, or in an exit before it.
type start struct {
	ready  bool
	status error
	errors string
}

// start waits for the readiness the unit's Type=notify waits for, or for the
// manager to exit first.
func (r *running) start(notify *net.UnixConn) start {
	r.t.Helper()
	deadline := time.Now().Add(readyDeadline)
	message := make([]byte, 256)
	for {
		if err := notify.SetReadDeadline(time.Now().Add(100 * time.Millisecond)); err != nil {
			r.t.Fatal(err)
		}
		length, _, err := notify.ReadFromUnix(message)
		switch {
		case err == nil:
			if slices.Contains(strings.Split(string(message[:length]), "\n"), "READY=1") {
				return start{ready: true}
			}
		case errors.Is(err, os.ErrDeadlineExceeded):
		default:
			r.t.Fatalf("reading readiness: %v", err)
		}
		select {
		case status := <-r.done:
			r.done <- status
			return start{status: status, errors: r.errors()}
		default:
		}
		if time.Now().After(deadline) {
			r.t.Fatalf("the manager neither became ready nor exited within %v", readyDeadline)
		}
	}
}

// waitReady waits for readiness; an exit before it fails the test.
func (r *running) waitReady(notify *net.UnixConn) {
	r.t.Helper()
	if s := r.start(notify); !s.ready {
		r.t.Fatalf("the manager exited (%v) before it was ready: %s", s.status, s.errors)
	}
}

// stop stops the manager as its unit does, with SIGTERM, and returns what it
// wrote to its standard error.
func (r *running) stop() string {
	r.t.Helper()
	if err := syscall.Kill(r.pid(), syscall.SIGTERM); err != nil {
		r.t.Fatal(err)
	}
	if status := <-r.done; status != nil {
		r.t.Fatalf("the manager's stop exited (%v): %s", status, r.errors())
	}
	return r.errors()
}

func (r *running) namespace() *namespaceID {
	id := identity(r.t, fmt.Sprintf("/proc/%d/ns/mnt", r.pid()))
	if id == nil {
		r.t.Fatal("no running manager")
	}
	return id
}

// kill ends the manager as a crash does: no drain and no stop-post recovery.
func (r *running) kill() {
	r.t.Helper()
	if err := syscall.Kill(r.pid(), syscall.SIGKILL); err != nil {
		r.t.Fatal(err)
	}
	if status := <-r.done; status == nil {
		r.t.Fatal("a killed manager exited successfully")
	}
}

// finish waits for a manager that exits by itself.
func (r *running) finish() (string, error) {
	status := <-r.done
	return r.errors(), status
}

// notifySocket binds the socket the manager reports readiness to.
func notifySocket(t *testing.T, path string) *net.UnixConn {
	t.Helper()
	conn, err := net.ListenUnixgram("unixgram", &net.UnixAddr{Name: path, Net: "unixgram"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { conn.Close() })
	return conn
}

func hostImage(t *testing.T) *imagetest.Image {
	t.Helper()
	architecture, ok := machinesproto.HostArchitecture()
	if !ok {
		t.Skip("no Cloud images for this architecture")
	}
	return imagetest.New(t, imagetest.Entries(), []imagetest.Executable{imagetest.Runner, imagetest.Tini}, architecture)
}

// A manager killed while it served leaves its namespace pinned by the handle. The
// next start recovers through it, releases it and becomes ready with its own; the
// unit's stop-post recovery of a killed manager releases the handle as well.
func TestTheNextStartRecoversAndReleasesAKilledManagersNamespace(t *testing.T) {
	roottest.Require(t)
	directory := t.TempDir()
	s := newSettings(t, directory, hostImage(t).Dir)
	notify := notifySocket(t, s.notify)
	h := startHost(t, offered)

	first := h.manager(s)
	first.waitReady(notify)
	pinned := first.namespace()
	if got := h.handle(); got == nil || *got != *pinned {
		t.Fatalf("a serving manager pins its namespace: handle %v, namespace %v", got, pinned)
	}
	data, err := os.ReadFile(h.runtimeFile(owner))
	if err != nil {
		t.Fatal(err)
	}
	var record map[string]string
	if err := json.Unmarshal(data, &record); err != nil || len(record) != 1 || record["dataDir"] != s.data {
		t.Fatalf("the owner record %s, %v", data, err)
	}
	first.kill()
	if got := h.handle(); got == nil || *got != *pinned {
		t.Fatalf("a killed manager's namespace stays pinned: %v", got)
	}

	second := h.manager(s)
	second.waitReady(notify)
	own := second.namespace()
	if *own == *pinned {
		t.Fatal("the second manager shares the first's namespace")
	}
	if got := h.handle(); got == nil || *got != *own {
		t.Fatalf("the next start released the handle it recovered through: %v", got)
	}
	second.kill()

	stderr, status := h.manager(s, "--recover").finish()
	if status != nil {
		t.Fatalf("the stop-post recovery exited (%v): %s", status, stderr)
	}
	if got := h.handle(); got != nil {
		t.Errorf("the stop-post recovery released the handle: %v", got)
	}
	if _, err := os.Stat(h.runtimeFile(owner)); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("the owner record stays: %v", err)
	}
}

// With the resource limits on, as by default, a host without the cgroup v2
// controllers stops the start before readiness, with an error that names every
// controller it lacks.
func TestAStartWithTheLimitsOnFailsNamingEachCgroupControllerTheHostLacks(t *testing.T) {
	roottest.Require(t)
	directory := t.TempDir()
	s := newSettings(t, directory, hostImage(t).Dir)
	notify := notifySocket(t, s.notify)
	h := startHost(t, absent)

	result := h.manager(s).start(notify)
	if result.ready {
		t.Fatal("the manager became ready without the cgroup controllers")
	}
	if result.status == nil {
		t.Error("the manager exited successfully")
	}
	if !strings.Contains(result.errors, "missing: cpu, memory, pids.") {
		t.Errorf("the controllers are not named: %s", result.errors)
	}
	if !strings.Contains(result.errors, "DEMI_MANAGED_LIMITS=off") {
		t.Errorf("the setting is not named: %s", result.errors)
	}
}

// With the resource limits off, the manager starts on a host where no cgroup can
// be created, and its log says the Clouds run without limits.
func TestAStartWithTheLimitsOffNeedsNoCgroupAndSaysSo(t *testing.T) {
	roottest.Require(t)
	directory := t.TempDir()
	s := newSettings(t, directory, hostImage(t).Dir)
	s.variables = append(s.variables, "DEMI_MANAGED_LIMITS=off")
	notify := notifySocket(t, s.notify)
	h := startHost(t, absent)

	manager := h.manager(s)
	manager.waitReady(notify)
	log := manager.stop()
	if !strings.Contains(log, "DEMI_MANAGED_LIMITS=off: Clouds run without CPU, memory or PID limits") {
		t.Errorf("the warning is missing:\n%s", log)
	}
	if !strings.Contains(log, "resource limits off") {
		t.Errorf("the readiness line does not name the mode:\n%s", log)
	}
}
