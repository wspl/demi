//go:build acceptance

package backend_test

import (
	"context"
	"fmt"
	"net"
	"net/netip"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/gates"
	browserplugin "github.com/wspl/demi/internal/plugins/browser"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

const (
	realFirst    = "8e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	realSecond   = "8e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02"
	realPatience = 240 * time.Second
)

// These tests deliberately do not call Parallel: starting a backend reconciles
// the shared manager and stops its Clouds.
type realCloud struct {
	*hostScenario
	data   string
	socket string
}

// realCloudStart binds the scenario backend to the supplied manager's allowed
// address and embedded command releases, using the existing conversation driver.
func realCloudStart(t *testing.T, brokenRunner bool) *realCloud {
	t.Helper()
	for _, name := range []string{
		"DEMI_TEST_MACHINES_SOCKET",
		"DEMI_TEST_CLOUD_URL",
		"DEMI_TEST_MACHINES_DATA",
		"DEMI_TEST_CLOUD_NATIVE",
	} {
		if os.Getenv(name) == "" {
			t.Skip("the Cloud suite: needs a machine manager and the suite's variables (scenarios.md § Cloud suite)")
		}
	}
	ctx, cancel := context.WithTimeout(t.Context(), 15*time.Minute)
	t.Cleanup(cancel)
	h, err := backendtest.NewHarness(ctx, t, os.Getenv("DEMI_TEST_MACHINES_SOCKET"))
	wireMust(t, err)
	address, err := url.Parse(os.Getenv("DEMI_TEST_CLOUD_URL"))
	wireMust(t, err)
	ip, err := netip.ParseAddr(address.Hostname())
	wireMust(t, err)
	port := address.Port()
	if port == "" {
		switch address.Scheme {
		case "http":
			port = "80"
		case "https":
			port = "443"
		default:
			t.Fatal("DEMI_TEST_CLOUD_URL must have an HTTP port")
		}
	}
	number, err := strconv.ParseUint(port, 10, 16)
	wireMust(t, err)
	h.Config.Address = netip.AddrPortFrom(ip, uint16(number))
	h.Config.PublicURL = address
	catalog, err := runners.PublishNative(ctx, os.Getenv("DEMI_TEST_CLOUD_NATIVE"))
	wireMust(t, err)
	t.Cleanup(func() { wireMust(t, catalog.Close(context.Background())) })
	h.Config.Native = catalog
	h.Config.Lifecycle.IdleWindow = 2 * time.Second
	h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	h.Config.Cloud.Sweep = 200 * time.Millisecond
	if brokenRunner {
		h.Config.Cloud.RunnerConnection = 30 * time.Second
	}
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	return &realCloud{
		&hostScenario{t, ctx, h, b, master, nil},
		os.Getenv("DEMI_TEST_MACHINES_DATA"),
		os.Getenv("DEMI_TEST_MACHINES_SOCKET"),
	}
}

// hold keeps a conversation's Cloud active through its normal file gate.
func (s *realCloud) hold(ctx context.Context, id string) *gates.Lease {
	s.t.Helper()
	gate, err := backendtest.FileGate(ctx, s.b.Backend, s.user.User.ID, webapi.ConversationID(id))
	wireMust(s.t, err)
	lease, err := gate.Enter(ctx, gates.Demand)
	wireMust(s.t, err)
	s.t.Cleanup(lease.Release)
	return lease
}

func (s *realCloud) device(ctx context.Context) string {
	s.t.Helper()
	observer := *s.hostScenario
	observer.ctx = ctx
	status := observer.cloudStatus()
	if status.Device == nil {
		s.t.Fatal("the Cloud is not allocated")
	}
	return string(status.Device.ID)
}

// until uses the shared status-change observer with Rust's per-transition hang guard.
func (s *realCloud) until(ctx context.Context, check func(webapi.CloudStatus) bool) webapi.CloudStatus {
	s.t.Helper()
	wait, cancel := context.WithTimeout(ctx, realPatience)
	defer cancel()
	observer := *s.hostScenario
	observer.ctx = wait
	return observer.cloudUntil(check)
}

func (s *realCloud) list(ctx context.Context, id string, status int) backendtest.Answer {
	s.t.Helper()
	return conversationRequest(ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+id+"/fs", "", status)
}

// realRun keeps the existing conversation driver's shell and vendor assertions.
func realRun(ctx context.Context, w *cloudWork, id, script string, watch ...int) string {
	w.s.t.Helper()
	wait, cancel := context.WithTimeout(ctx, realPatience)
	defer cancel()
	owner := w.s
	scenario := *owner
	scenario.ctx = wait
	w.s = &scenario
	defer func() { w.s = owner }()
	milliseconds := 240000
	if len(watch) != 0 {
		milliseconds = watch[0]
	}
	return w.turn(id, script, "done", milliseconds)
}

func realContains(t *testing.T, output string, expected ...string) {
	t.Helper()
	for _, want := range expected {
		if !strings.Contains(output, want) {
			t.Fatalf("missing %q: %s", want, output)
		}
	}
}

func realMeasured(t *testing.T, what string, start time.Time) {
	t.Helper()
	t.Logf("cloud-suite measurement: %s: %s: %d ms", t.Name(), what, time.Since(start).Milliseconds())
}

// image reads a committed generation even after the backend has closed. Cancel
// the observation client before Close so it cannot reconcile the shared manager.
func (s *realCloud) image(ctx context.Context, device string) backendtest.RealImageState {
	s.t.Helper()
	state, err := backendtest.RealImage(ctx, s.socket, device)
	wireMust(s.t, err)
	if state == nil {
		s.t.Fatal("the Cloud has no committed generation")
	}
	return *state
}

func (s *realCloud) generation(device string, state backendtest.RealImageState) string {
	return filepath.Join(s.data, "images", device, "generations", string(state.Generation))
}

// realCommand runs the Cloud suite's host-side image inspection tools and waits
// for each process; stderr is retained for a failed assertion.
func realCommand(ctx context.Context, t *testing.T, program string, args ...string) string {
	t.Helper()
	command := exec.CommandContext(ctx, program, args...)
	var stderr strings.Builder
	command.Stderr = &stderr
	output, err := command.Output()
	if err != nil {
		t.Fatalf("%s %v: %v: %s", program, args, err, stderr.String())
	}
	return string(output)
}

func realCapacity(ctx context.Context, t *testing.T, image string) uint64 {
	t.Helper()
	output := realCommand(ctx, t, "dumpe2fs", "-h", image)
	field := func(name string) uint64 {
		for _, line := range strings.Split(output, "\n") {
			if value, ok := strings.CutPrefix(line, name); ok {
				number, err := strconv.ParseUint(strings.TrimSpace(value), 10, 64)
				wireMust(t, err)
				return number
			}
		}
		t.Fatalf("dumpe2fs names no %s: %s", name, output)
		return 0
	}
	return field("Block count:") * field("Block size:")
}

// boot validates the manager's own persisted sandbox contract at entry.
func (s *realCloud) boot(device string) string {
	s.t.Helper()
	id, err := backendtest.RealBootID(s.data, device)
	wireMust(s.t, err)
	return id
}

func (s *realCloud) memory(device string) {
	s.t.Helper()
	boot := s.boot(device)
	entries, err := os.ReadDir("/proc")
	wireMust(s.t, err)
	type peak struct {
		count int
		kb    uint64
	}
	peaks := map[string]peak{}
	for _, entry := range entries {
		if _, err := strconv.ParseUint(entry.Name(), 10, 32); err != nil {
			continue
		}
		command, commandErr := os.ReadFile(filepath.Join("/proc", entry.Name(), "cmdline"))
		status, statusErr := os.ReadFile(filepath.Join("/proc", entry.Name(), "status"))
		// A process that ended between the listing and reads is not measured.
		if commandErr != nil || statusErr != nil {
			continue
		}
		if !strings.Contains(string(command), boot) {
			continue
		}
		program := strings.Split(string(command), "\x00")[0]
		peak := peaks[program]
		peak.count++
		for _, line := range strings.Split(string(status), "\n") {
			if value, ok := strings.CutPrefix(line, "VmHWM:"); ok {
				kb, err := strconv.ParseUint(
					strings.TrimSpace(strings.TrimSuffix(strings.TrimSpace(value), "kB")),
					10,
					64,
				)
				if err == nil {
					peak.kb = max(peak.kb, kb)
				}
			}
		}
		peaks[program] = peak
	}
	var programs []string
	for program := range peaks {
		programs = append(programs, program)
	}
	slices.Sort(programs)
	for _, program := range programs {
		peak := peaks[program]
		s.t.Logf(
			"cloud-suite measurement: %s: %s: %d processes, largest peak %d MiB",
			s.t.Name(),
			program,
			peak.count,
			peak.kb/1024,
		)
	}
}

const realPackage = `mkdir -p package/DEBIAN package/usr/share/cloud-suite
printf 'Package: cloud-suite-probe\nVersion: 1.0\nArchitecture: all` +
	`\nMaintainer: Cloud suite <suite@example.test>` +
	`\nDescription: what the Cloud suite installs\n' > package/DEBIAN/control
echo installed > package/usr/share/cloud-suite/marker
dpkg-deb --root-owner-group --build package probe.deb > /dev/null
sudo -n dpkg -i probe.deb > /dev/null`

// TestACloudRunsAsUID1000ForEveryConversationAndKeepsASystemPackageAndHomeAcrossAStop
// checks Cloud identity and package and home persistence across a stop.
// Tens of seconds: a Cloud boots, installs a local package, stops, wakes and saves.
func TestACloudRunsAsUID1000ForEveryConversationAndKeepsASystemPackageAndHomeAcrossAStop(t *testing.T) {
	s := realCloudStart(t, false)
	first, second := s.work(realFirst), s.work(realSecond)
	working := s.hold(s.ctx, realFirst)
	started := time.Now()
	s.list(s.ctx, realFirst, 200)
	realMeasured(t, "first boot until the runner is ready", started)
	device := s.device(s.ctx)
	s.memory(device)
	started = time.Now()
	facts := realRun(
		s.ctx,
		first,
		"facts",
		"set -e\nid -u\nid -g\nsudo -n id -u\necho home-data > ~/note\n"+realPackage+
			"\ndpkg-query -W -f='${Status}\\n' cloud-suite-probe\ngrep MemTotal /proc/meminfo",
	)
	realMeasured(t, "first command", started)
	realContains(t, facts, "1000\n1000\n0\n", "install ok installed")
	memory := "MemTotal: unknown"
	for _, line := range strings.Split(facts, "\n") {
		if strings.HasPrefix(line, "MemTotal:") {
			memory = line
			break
		}
	}
	t.Logf("cloud-suite measurement: %s: the Cloud's %s", t.Name(), memory)
	upload := "/api/conversations/" + realFirst + "/fs/raw?path=/home/demi/sessions/" + realFirst + "/uploaded"
	response, err := s.b.Response(s.ctx, "PUT", upload, &s.user, nil, strings.NewReader("from-api"))
	wireMust(t, err)
	uploaded, err := backendtest.ReadAnswer(s.ctx, response)
	wireMust(t, err)
	if uploaded.Status != 204 {
		t.Fatalf("upload: %d %s", uploaded.Status, uploaded.Body)
	}
	seen := realRun(
		s.ctx,
		second,
		"seen",
		fmt.Sprintf(
			"stat -c %%u:%%g /home/demi/sessions/%s/uploaded; cat /home/demi/sessions/%s/uploaded; echo; cat ~/note",
			realFirst,
			realFirst,
		),
	)
	realContains(t, seen, "1000:1000\nfrom-api\nhome-data")
	conversationEqual(t, s.device(s.ctx), device)
	started = time.Now()
	working.Release()
	s.until(s.ctx, func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	realMeasured(t, "idle stop, window included", started)
	stopped := s.image(s.ctx, device)
	output := realCommand(
		s.ctx,
		t,
		"debugfs",
		"-R",
		"ls -p /upper/var/lib/demi/jobs",
		filepath.Join(s.generation(device, stopped), "system.ext4"),
	)
	var jobs []string
	for _, line := range strings.Split(output, "\n") {
		fields := strings.Split(line, "/")
		if len(fields) > 5 && fields[5] != "" && fields[5] != "." && fields[5] != ".." {
			jobs = append(jobs, fields[5])
		}
	}
	if !slices.Contains(jobs, "edits.lock") || slices.Contains(jobs, realFirst) || slices.Contains(jobs, realSecond) {
		t.Fatal(jobs)
	}
	working = s.hold(s.ctx, realFirst)
	started = time.Now()
	s.list(s.ctx, realFirst, 200)
	realMeasured(t, "wake until the runner is ready", started)
	woken := realRun(
		s.ctx,
		first,
		"woken",
		"cat ~/note; dpkg-query -W -f='${Status}\\n' cloud-suite-probe; cat /usr/share/cloud-suite/marker",
	)
	realContains(t, woken, "home-data\ninstall ok installed\ninstalled")
	conversationEqual(t, s.device(s.ctx), device)
	s.memory(device)
	working.Release()
	wireMust(t, s.b.Close(s.ctx))
	saved := s.image(s.ctx, device)
	conversationEqual(
		t,
		saved.HomeBytes,
		realCapacity(s.ctx, t, filepath.Join(s.generation(device, saved), "home.ext4")),
	)
}

// TestACloudGrowsItsHomeOnlineAndItsSavedGenerationRecordsTheGrownCapacity
// checks online home growth and saved capacity.
// Tens of seconds to minutes: boot, fill, idle stop, wake, online growth and save.
// Requires the manager's CAP_SYS_RESOURCE, as in the Rust scenario.
func TestACloudGrowsItsHomeOnlineAndItsSavedGenerationRecordsTheGrownCapacity(t *testing.T) {
	s := realCloudStart(t, false)
	first := s.work(realFirst)
	working := s.hold(s.ctx, realFirst)
	filled := realRun(s.ctx, first, "fill", "fallocate -l 800M ~/fill && df -B1 --output=size /home | tail -1")
	var initial uint64
	for _, line := range strings.Split(filled, "\n") {
		if value, err := strconv.ParseUint(strings.TrimSpace(line), 10, 64); err == nil {
			initial = value
			break
		}
	}
	if initial == 0 {
		t.Fatalf("no size: %s", filled)
	}
	device := s.device(s.ctx)
	working.Release()
	s.until(s.ctx, func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	working = s.hold(s.ctx, realFirst)
	s.list(s.ctx, realFirst, 200)
	started := time.Now()
	grown := realRun(
		s.ctx,
		first,
		"grown",
		fmt.Sprintf(
			"for second in $(seq 180); do size=$(df -B1 --output=size /home | tail -1); "+
				"[ $size -gt %d ] && break; sleep 1; done; echo home-size $size",
			initial,
		),
	)
	realMeasured(t, "growth after the wake", started)
	var size uint64
	for _, line := range strings.Split(grown, "\n") {
		if value, ok := strings.CutPrefix(line, "home-size "); ok {
			parsed, err := strconv.ParseUint(strings.TrimSpace(value), 10, 64)
			wireMust(t, err)
			size = parsed
			break
		}
	}
	if size <= initial {
		t.Fatalf("home stayed at %d bytes: %s", size, grown)
	}
	working.Release()
	wireMust(t, s.b.Close(s.ctx))
	saved := s.image(s.ctx, device)
	conversationEqual(
		t,
		saved.HomeBytes,
		realCapacity(s.ctx, t, filepath.Join(s.generation(device, saved), "home.ext4")),
	)
	if saved.HomeBytes <= initial {
		t.Fatal(saved)
	}
}

// TestAResetBringsBackACloudWhoseBashOrRunnerIsBrokenAndKeepsItsHome
// checks reset recovery with a broken shell or runner.
// Tens of seconds: two resets and one failed boot, with a 30-second runner guard.
func TestAResetBringsBackACloudWhoseBashOrRunnerIsBrokenAndKeepsItsHome(t *testing.T) {
	s := realCloudStart(t, true)
	first := s.work(realFirst)
	working := s.hold(s.ctx, realFirst)
	realContains(
		t,
		realRun(s.ctx, first, "break-bash", "echo latest-home > ~/note; sudo -n chmod 000 /usr/bin/bash; echo broken"),
		"broken",
	)
	device := s.device(s.ctx)
	working.Release()
	started := time.Now()
	s.resetCloud("7e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01")
	ready := func(status webapi.CloudStatus) bool {
		return status.Operation != nil && status.Operation.Phase == webapi.ResetPhaseReady
	}
	s.until(s.ctx, ready)
	realMeasured(t, "reset until ready", started)
	working = s.hold(s.ctx, realFirst)
	realContains(t, realRun(s.ctx, first, "repaired", "cat ~/note; stat -c %a /usr/bin/bash"), "latest-home\n755")
	realContains(t, realRun(s.ctx, first, "remove-runner", "sudo -n rm /usr/bin/demi-runner; echo removed"), "removed")
	working.Release()
	s.until(s.ctx, func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	conversationRefusal(t, s.list(s.ctx, realFirst, 503), webapi.ErrorCodeCloudUnavailable)
	s.resetCloud("7e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02")
	s.until(s.ctx, ready)
	working = s.hold(s.ctx, realFirst)
	realContains(
		t,
		realRun(s.ctx, first, "back", "cat ~/note; test -x /usr/bin/demi-runner && echo runner-back"),
		"latest-home\nrunner-back",
	)
	conversationEqual(t, s.device(s.ctx), device)
	working.Release()
	wireMust(t, s.b.Close(s.ctx))
}

const realMapper = `import mmap, os, sys, time
mark = sys.argv[2].encode()
descriptor = os.open(sys.argv[1], os.O_RDWR | os.O_CREAT | os.O_TRUNC, 0o644)
os.ftruncate(descriptor, 4096)
mapping = mmap.mmap(descriptor, 4096, mmap.MAP_SHARED, mmap.PROT_READ | mmap.PROT_WRITE)
mapping[:len(mark)] = mark
print("mapped", flush=True)
time.sleep(600)
`

func (s *realCloud) tabs(ctx context.Context, pagePath string) []browserop.BrowserTab {
	s.t.Helper()
	// The ignored Rust scenario calls a nonexistent tabs method. Like the page,
	// read conversation state through its declared endpoint instead.
	answer := conversationRequest(ctx, s.t, s.b, &s.user, "GET", pagePath+"/state", "", 200)
	state := conversationDecode(s.t, answer, webapi.DecodePluginStateAnswer)
	tabs, err := browserplugin.DecodeBrowserTabs(state.State)
	wireMust(s.t, err)
	return tabs.Tabs
}

// openTab opens through the page call and observes the title in page state.
func (s *realCloud) openTab(ctx context.Context, pagePath, page string) {
	s.t.Helper()
	params, err := (browserplugin.OpenTab{URL: &page}).MarshalJSON()
	wireMust(s.t, err)
	answer, err := s.b.Post(ctx, pagePath+"/calls/open", &s.user, params)
	wireMust(s.t, err)
	if answer.Status < 200 || answer.Status >= 300 {
		s.t.Fatalf("open: %d %s", answer.Status, answer.Body)
	}
	opened := conversationDecode(s.t, answer, browserplugin.DecodeOpenedTab)
	wait, cancel := context.WithTimeout(ctx, realPatience)
	defer cancel()
	for {
		for _, tab := range s.tabs(wait, pagePath) {
			if tab.ID == opened.Tab.ID && tab.Title == "cloud-suite page" {
				return
			}
		}
	}
}

// TestACheckpointWithChromeOpenSavesBothImagesWithWhatAMappingWroteAndKeepsEveryProcessUntilTheIdleConversationsRelease
// checks checkpoint persistence with Chrome and mapped files.
// Tens of seconds: Chrome boots, two images checkpoint, and a conversation releases.
func TestACheckpointWithChromeOpenSavesBothImagesWithWhatAMappingWroteAndKeepsEveryProcessUntilTheIdleConversationsRelease(
	t *testing.T,
) {
	s := realCloudStart(t, false)
	first := s.work(realFirst)
	working := s.hold(s.ctx, realFirst)
	session := "/home/demi/sessions/" + realFirst
	script := "cat > page.html <<'HTML'\n<!doctype html><title>cloud-suite page</title><p>Chrome on the Cloud</p>" +
		"\nHTML\ncat > mapper.py <<'PY'\n" + realMapper + "PY\necho written"
	realContains(t, realRun(s.ctx, first, "page", script), "written")
	realContains(
		t,
		realRun(s.ctx, first, "mapper", "python3 mapper.py mapped 'written through a mapping'", 5000),
		"mapped",
	)
	pagePath := "/api/conversations/" + realFirst + "/plugins/browser"
	started := time.Now()
	s.openTab(s.ctx, pagePath, "file://"+session+"/page.html")
	realMeasured(t, "first Chrome tab until its page shows", started)
	started = time.Now()
	s.openTab(s.ctx, pagePath, "file://"+session+"/page.html")
	realMeasured(t, "later Chrome tab until its page shows", started)
	realContains(
		t,
		realRun(
			s.ctx,
			first,
			"renderers",
			"for pid in $(pgrep -f -- '--type=renderer'); do grep '^Seccomp:' /proc/$pid/status; done",
		),
		"Seccomp:\t2",
	)
	device := s.device(s.ctx)
	s.memory(device)
	before := s.image(s.ctx, device)
	started = time.Now()
	err := backendtest.RealCheckpoint(s.ctx, s.b, device)
	wireMust(t, err)
	realMeasured(t, "checkpoint", started)
	after := s.image(s.ctx, device)
	if after.Generation == before.Generation {
		t.Fatal("checkpoint kept generation")
	}
	generation := s.generation(device, after)
	system, home := filepath.Join(generation, "system.ext4"), filepath.Join(generation, "home.ext4")
	for _, path := range []string{system, home} {
		info, err := os.Stat(path)
		wireMust(t, err)
		if !info.Mode().IsRegular() {
			t.Fatal(path)
		}
	}
	saved := realCommand(s.ctx, t, "debugfs", "-R", "cat /demi/sessions/"+realFirst+"/mapped", home)
	if !strings.HasPrefix(saved, "written through a mapping") {
		t.Fatalf("saved: %q", saved)
	}
	copies := t.TempDir()
	for _, image := range []string{system, home} {
		copyPath := filepath.Join(copies, "copy.ext4")
		realCommand(s.ctx, t, "cp", "--reflink=auto", "--sparse=always", image, copyPath)
		file, err := os.Open(copyPath)
		wireMust(t, err)
		syncErr := file.Sync()
		closeErr := file.Close()
		wireMust(t, syncErr)
		wireMust(t, closeErr)
		blocks := func(path string) uint64 {
			output := realCommand(s.ctx, t, "stat", "-c", "%b", path)
			number, err := strconv.ParseUint(strings.TrimSpace(output), 10, 64)
			wireMust(t, err)
			return number
		}
		original, copied := blocks(image), blocks(copyPath)
		if original > copied {
			t.Fatalf("%s: %d blocks, cp's %d", image, original, copied)
		}
		wireMust(t, os.Remove(copyPath))
	}
	conversationEqual(t, len(s.tabs(s.ctx, pagePath)), 2)
	realContains(
		t,
		realRun(s.ctx, first, "alive", "pgrep -f mapper.py > /dev/null && echo mapper-alive"),
		"mapper-alive",
	)
	second := s.work(realSecond)
	busy := s.hold(s.ctx, realSecond)
	realContains(t, realRun(s.ctx, first, "end", "pkill -f mapper.py; echo ended"), "ended")
	started = time.Now()
	working.Release()
	wait, cancel := context.WithTimeout(s.ctx, realPatience)
	defer cancel()
	for len(s.tabs(wait, pagePath)) != 0 {
		wireMust(t, wait.Err())
	}
	realMeasured(t, "release of an idle conversation, window included", started)
	realContains(t, realRun(s.ctx, second, "held", "echo held"), "held")
	conversationEqual(t, s.cloudStatus().State, webapi.CloudStateRunning)
	busy.Release()
	wireMust(t, s.b.Close(s.ctx))
}

const realProbe = `import socket, sys
for line in sys.stdin:
    name, host, port = line.split()
    try:
        socket.create_connection((host, int(port)), timeout=3).close()
        print(name, "reached")
    except OSError:
        print(name, "refused")
`

// TestTwoUsersCloudsRunAtOnceAndReachTheBackendButNothingElsePrivate
// checks Cloud concurrency and isolation between users.
// Tens of seconds: two Clouds boot; each refused connection has a three-second guard.
func TestTwoUsersCloudsRunAtOnceAndReachTheBackendButNothingElsePrivate(t *testing.T) {
	s := realCloudStart(t, false)
	wireMust(t, s.h.AddUser(s.ctx, "ana@example.test", "ana-pass-1", webapi.RoleUser))
	ana, err := s.b.Login(s.ctx, "ana@example.test", "ana-pass-1")
	wireMust(t, err)
	first := s.work(realFirst)
	otherScenario := *s.hostScenario
	otherScenario.user = ana
	other := &realCloud{&otherScenario, s.data, s.socket}
	// Shared-instance provider entries are created by the master for both users.
	vendor := providertest.StartVendor(t)
	provider := conversationAnthropic(s.ctx, t, s.b, &s.user, vendor)
	conversationCreate(s.ctx, t, s.b, &ana, realSecond)
	second := &cloudWork{s: &otherScenario, vendor: vendor, provider: provider, id: realSecond}
	second.open()
	working, alsoWorking := s.hold(s.ctx, realFirst), other.hold(s.ctx, realSecond)
	serving := realRun(
		s.ctx,
		second,
		"serve",
		"ip -4 -o addr show scope global | awk '{print $4}'; exec python3 -m http.server 8123",
		5000,
	)
	var otherIP netip.Addr
	for _, line := range strings.Split(serving, "\n") {
		if prefix, err := netip.ParsePrefix(strings.TrimSpace(line)); err == nil && prefix.Addr().Is4() {
			otherIP = prefix.Addr()
			break
		}
	}
	if !otherIP.IsValid() {
		t.Fatalf("no address: %s", serving)
	}
	s.list(s.ctx, realFirst, 200)
	conversationEqual(t, s.cloudStatus().State, webapi.CloudStateRunning)
	conversationEqual(t, other.cloudStatus().State, webapi.CloudStateRunning)
	if s.device(s.ctx) == other.device(s.ctx) {
		t.Fatal("users share a Cloud")
	}
	address := s.b.Address()
	private, err := (&net.ListenConfig{}).Listen(s.ctx, "tcp", net.JoinHostPort(address.Addr().String(), "0"))
	wireMust(t, err)
	defer func() { wireMust(t, private.Close()) }()
	_, port, err := net.SplitHostPort(private.Addr().String())
	wireMust(t, err)
	targets := fmt.Sprintf(
		"backend %s %d\nhost-service %s %s"+
			"\nprivate 10.0.0.1 80\nmetadata 169.254.169.254 80\nother-cloud %s 8123"+
			"\nipv6 fd00::1 80\n",
		address.Addr(),
		address.Port(),
		address.Addr(),
		port,
		otherIP,
	)
	probed := realRun(
		s.ctx,
		first,
		"probe",
		"cat > probe.py <<'PY'\n"+realProbe+"PY\npython3 probe.py <<'TARGETS'\n"+targets+
			"TARGETS\necho ipv6-addresses $(ip -6 addr show scope global | wc -l)",
	)
	realContains(
		t,
		probed,
		"backend reached",
		"host-service refused",
		"private refused",
		"metadata refused",
		"other-cloud refused",
		"ipv6 refused",
		"ipv6-addresses 0",
	)
	s.memory(s.device(s.ctx))
	working.Release()
	alsoWorking.Release()
	wireMust(t, s.b.Close(s.ctx))
}

// TestACloudWhoseSandboxIsKilledReportsADeathAndBootsAgainWithItsFiles
// checks recovery with retained files after sandbox death.
// Tens of seconds: boot, kill the Sentry, observe death, and boot with saved files.
func TestACloudWhoseSandboxIsKilledReportsADeathAndBootsAgainWithItsFiles(t *testing.T) {
	s := realCloudStart(t, false)
	first := s.work(realFirst)
	working := s.hold(s.ctx, realFirst)
	realContains(t, realRun(s.ctx, first, "note", "echo before-death > ~/note; sync; echo wrote"), "wrote")
	device := s.device(s.ctx)
	boot := s.boot(device)
	realCommand(s.ctx, t, "pkill", "-KILL", "-f", "^runsc-sandbox .*"+boot)
	started := time.Now()
	s.until(s.ctx, func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	realMeasured(t, "death until off", started)
	started = time.Now()
	back := realRun(s.ctx, first, "back", "cat ~/note; ls -d /var/lib/demi/jobs/job-* | wc -l")
	realMeasured(t, "boot after the death and command", started)
	realContains(t, back, "before-death\n1")
	working.Release()
	wireMust(t, s.b.Close(s.ctx))
}
