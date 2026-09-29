package backendtest_test

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// The conversation ids the Cloud scenarios create, and a reset's id, as the page
// names it.
const (
	cloudFirst  = "4e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	cloudSecond = "4e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02"
	cloudReset  = "6e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a09"
)

// cloudStatus is the user's Cloud, as GET /api/cloud reads it.
func cloudStatus(b *backendtest.Backend, session *backendtest.Session) map[string]any {
	return b.Get("/api/cloud", session).Expect(http.StatusOK).Value().(map[string]any)
}

// untilCloud returns the Cloud's status once check holds.
func untilCloud(t *testing.T, b *backendtest.Backend, session *backendtest.Session, what string, check func(map[string]any) bool) map[string]any {
	t.Helper()
	var status map[string]any
	backendtest.Eventually(t, what, func() bool {
		status = cloudStatus(b, session)
		return check(status)
	})
	return status
}

func resetIs(phase string) func(map[string]any) bool {
	return func(status map[string]any) bool {
		return backendtest.At(status, "operation.phase") == phase
	}
}

// cloudOnline reports whether the user's Cloud has a live runner, as the device
// list says.
func cloudOnline(b *backendtest.Backend, session *backendtest.Session) bool {
	for _, device := range b.Devices(session) {
		if device["kind"] == "managed" && device["online"] == true {
			return true
		}
	}
	return false
}

// resetCloud asks for the reset with the id, and answers the operation.
func resetCloud(b *backendtest.Backend, session *backendtest.Session, id string) *backendtest.Answer {
	return b.Post("/api/cloud/reset", session, backendtest.Map{"operationId": id}).Expect(http.StatusAccepted)
}

func cloudShell(id, script string) *scripted.Response {
	return backendtest.ShellCall(id, script, 20*time.Second)
}

func resetCalls(h *backendtest.Harness) []string {
	var resets []string
	for _, call := range h.Manager.Calls() {
		if strings.HasPrefix(call, "reset:") {
			resets = append(resets, call)
		}
	}
	return resets
}

// Cost: one backend started twice, a scripted manager and a Cloud's runner,
// about a second.
func TestAConversationArchivedWhileItsCloudIsStoppedDoesNotWakeIt(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "/a")
	b.CreateConversation(master, cloudFirst)
	work := b.Open(master, vendor, cloudFirst, provider, "/a")
	ran := work.Turn(backendtest.ShellCall("t1", "echo ran", 20*time.Second), backendtest.Say("ran"))
	contains(t, ran.Result(t, 0), "exitCode: 0")
	device := theCloud(t, h)

	// The backend's close stops the Cloud. Archived while the Cloud is stopped,
	// the conversation's release does not wake it: the stop ended what a release
	// would.
	b.Stop()
	b = h.Start()
	b.Patch("/api/conversations/"+cloudFirst, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	if got := h.Manager.Count("wake:" + device); got != 1 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	if state := cloudStatus(b, master)["state"]; state != "off" {
		t.Fatalf("the Cloud is %v", state)
	}
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner booted twice, over
// a second, and the builtin package installed.
func TestACloudTheManagerStoppedWithoutAWordBootsAgainForTheOperationsThatNeedIt(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "/a")
	b.CreateConversation(master, cloudFirst)
	work := b.Open(master, vendor, cloudFirst, provider, "/a")
	wrote := work.Turn(cloudShell("t1", "echo retained > note"), backendtest.Say("saved"))
	contains(t, wrote.Result(t, 0), "exitCode: 0")
	device := theCloud(t, h)
	home := h.Manager.Home(device)

	// A manager restart stops the sandbox before it can report the stop.
	h.Manager.StopQuietly(device)
	backendtest.Eventually(t, "the backend sees the runner go", func() bool { return !cloudOnline(b, master) })
	if state := cloudStatus(b, master)["state"]; state != "running" {
		t.Fatalf("the Cloud is %v", state)
	}

	// Two reads at once join one recovery, which boots the Cloud again.
	path := "/api/conversations/" + cloudFirst + "/fs/file?" + query("path", home+"/sessions/"+cloudFirst+"/note")
	backendtest.Concurrently(2, func(int) {
		read, err := b.TryDo(backendtest.Request{Path: path, Session: master})
		if err != nil || read.Status != http.StatusOK || read.Str("text") != "retained\n" {
			t.Errorf("the read is %v %v", read, err)
		}
	})
	if got := h.Manager.Count("wake:" + device); got != 2 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	b.Stop()
}

// Cost: one backend, a scripted manager, a Cloud's runner booted three times
// and a paired device, several seconds: idle windows of 0.8 s pass in real time,
// and the Cloud and the device each install the builtin package.
func TestAConversationThatLeftTheCloudReachesItAsAnAttachedHostWhichItsWorkWakes(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	idleAfter(h, 800*time.Millisecond)
	// The conversation's own idle watch reads its activity once an hour while it
	// works here, so the Cloud's own idle watch is what stops it.
	h.Lifecycle().IdlePollMs = backendtest.Ptr(uint64(3_600_000))
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	alpha := b.Pair(master, "alpha")
	writeFile(t, filepath.Join(alpha.Home(), "notes.txt"), "on alpha\n")
	provider := b.Anthropic(master, vendor, "/work")
	b.CreateConversation(master, cloudFirst)
	work := b.Open(master, vendor, cloudFirst, provider, "/work")

	// On the Cloud, with alpha attached: the Cloud's job reaches alpha.
	attached := b.Post("/api/conversations/"+cloudFirst+"/hosts", master, backendtest.Map{"deviceId": alpha.ID()})
	if attached.Status/100 != 2 {
		t.Fatalf("attaching alpha answers %d: %s", attached.Status, attached.Body)
	}
	script := "echo report > report.txt && demi host shell --host alpha 'cat notes.txt'"
	fromCloud := work.Turn(backendtest.ShellCall("t1", script, 30*time.Second), backendtest.Say("reached alpha"))
	contains(t, fromCloud.Result(t, 0), "on alpha")
	device := theCloud(t, h)
	session := h.Manager.Home(device) + "/sessions/" + cloudFirst

	// Leaving the Cloud keeps it attached where the conversation worked.
	b.SwitchTo(master, cloudFirst, alpha, alpha.Home())
	hosts, _ := b.Get("/api/conversations/"+cloudFirst+"/hosts", master).At("hosts").([]any)
	var cloud any
	for _, host := range hosts {
		if backendtest.At(host, "deviceId") == device {
			cloud = host
		}
	}
	if cloud == nil || backendtest.At(cloud, "name") != "Cloud" || backendtest.At(cloud, "cwd") != session {
		t.Fatalf("the Cloud does not stay attached: %v", hosts)
	}

	// The Cloud stops once no conversation uses it; browsing it wakes it, and so
	// does a command sent to it.
	backendtest.Eventually(t, "the idle Cloud stops", func() bool { return !h.Manager.Running(device) })
	browsed := b.Get("/api/conversations/"+cloudFirst+"/hosts/"+device+"/fs?"+query("path", session), master).Expect(http.StatusOK)
	var names []string
	entries, _ := browsed.At("entries").([]any)
	for _, entry := range entries {
		names = append(names, backendtest.At(entry, "name").(string))
	}
	if !slices.Contains(names, "report.txt") {
		t.Fatalf("the Cloud lists %v", names)
	}
	if got := h.Manager.Count("wake:" + device); got != 2 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	backendtest.Eventually(t, "the idle Cloud stops again", func() bool { return !h.Manager.Running(device) })
	back := work.Turn(backendtest.ShellCall("t2", "demi host shell --host Cloud 'cat report.txt'", 30*time.Second), backendtest.Say("read the Cloud"))
	contains(t, back.FirstRequest(), "[Execution target switched]")
	contains(t, back.Result(t, 0), "report")
	if got := h.Manager.Count("wake:" + device); got != 3 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	backendtest.Eventually(t, "the idle Cloud stops a third time", func() bool { return !h.Manager.Running(device) })
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, several seconds:
// the Cloud installs the builtin package, and boots again after the reset.
func TestAResetKeepsTheCloudsFilesAndIdentityTellsTheModelAndIsTheSameResetWhenAskedAgain(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "/a")
	b.CreateConversation(master, cloudFirst)
	work := b.Open(master, vendor, cloudFirst, provider, "/a")
	wrote := work.Turn(cloudShell("t1", "echo retained > note"), backendtest.Say("written"))
	contains(t, wrote.Result(t, 0), "exitCode: 0")
	before := cloudStatus(b, master)["device"]
	if before == nil {
		t.Fatal("the Cloud was not made")
	}

	backendtest.Concurrently(3, func(int) {
		answer, err := b.TryDo(backendtest.Request{Method: http.MethodPost, Path: "/api/cloud/reset", Session: master, Body: backendtest.Map{"operationId": cloudReset}})
		if err != nil || answer.Status != http.StatusAccepted || answer.Str("operation.id") != cloudReset {
			t.Errorf("the reset answers %v %v", answer, err)
		}
	})
	ready := untilCloud(t, b, master, "the reset is ready", resetIs("ready"))
	if ready["state"] != "running" {
		t.Fatalf("the Cloud is %v", ready["state"])
	}
	backendtest.AssertJSON(t, ready["device"], before)

	// Home stays; the model hears of the reset before its next request.
	next := work.Turn(cloudShell("t2", "cat note"), backendtest.Say("read"))
	contains(t, next.Result(t, 0), "retained")
	contains(t, next.FirstRequest(), "[Cloud reset "+cloudReset+"]")
	if again := resetCloud(b, master, cloudReset); again.Str("operation.phase") != "ready" {
		t.Fatalf("the same reset again answers %s", again.Body)
	}
	device := theCloud(t, h)
	if resets := resetCalls(h); len(resets) != 1 || resets[0] != fmt.Sprintf("reset:%s:%s:test-base", device, cloudReset) {
		t.Fatalf("the manager was asked %v", resets)
	}

	// Another reset while one runs is refused.
	hold := scripted.NewResetHold()
	h.Manager.Configure(func(script *scripted.Script) { script.HoldReset = hold })
	const second = "7e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a0a"
	resetCloud(b, master, second)
	<-hold.Arrived()
	third := b.Post("/api/cloud/reset", master, backendtest.Map{"operationId": "8e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a0b"})
	wantRefusal(t, third, http.StatusConflict, "cloud_resetting", "a reset while one runs")
	hold.Proceed()
	untilCloud(t, b, master, "the second reset is ready", func(status map[string]any) bool {
		return backendtest.At(status, "operation.id") == second && backendtest.At(status, "operation.phase") == "ready"
	})
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, about two seconds:
// the Cloud installs the builtin package, and the resumed reset boots it again.
func TestAFailedResetReportsItsFailureAndTheSameOperationResumesWithoutLosingHome(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "/a")
	b.CreateConversation(master, cloudFirst)
	work := b.Open(master, vendor, cloudFirst, provider, "/a")
	wrote := work.Turn(cloudShell("t1", "printf retained > note"), backendtest.Say("written"))
	contains(t, wrote.Result(t, 0), "exitCode: 0")
	device := theCloud(t, h)

	failure := "image publication unavailable"
	h.Manager.Configure(func(script *scripted.Script) { script.FailReset = &failure })
	resetCloud(b, master, cloudReset)
	failed := untilCloud(t, b, master, "the reset fails", resetIs("failed"))
	if got, _ := backendtest.At(failed, "operation.error").(string); !strings.Contains(got, failure) {
		t.Fatalf("the reset's error is %q", got)
	}
	if failed["state"] != "off" {
		t.Fatalf("the Cloud is %v", failed["state"])
	}

	resetCloud(b, master, cloudReset)
	untilCloud(t, b, master, "the resumed reset is ready", resetIs("ready"))
	path := h.Manager.Home(device) + "/sessions/" + cloudFirst + "/note"
	read := b.Get("/api/conversations/"+cloudFirst+"/fs/file?"+query("path", path), master)
	if read.Str("text") != "retained" {
		t.Fatalf("the note reads %s", read.Body)
	}
	// The retry reset on the base the first attempt selected.
	want := fmt.Sprintf("reset:%s:%s:test-base", device, cloudReset)
	if resets := resetCalls(h); !slices.Equal(resets, []string{want, want}) {
		t.Fatalf("the manager was asked %v", resets)
	}
	b.Stop()
}

// Cost: one backend and a scripted manager, about a second.
func TestACloudThatKeepsDyingStopsBootingByItselfUntilAResetStartsIt(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	b.CreateConversation(master, cloudFirst)
	listing := "/api/conversations/" + cloudFirst + "/fs"
	for death := 1; death <= 3; death++ {
		b.Get(listing, master).Expect(http.StatusOK)
		h.Manager.Kill(theCloud(t, h))
		untilCloud(t, b, master, fmt.Sprintf("death %d stops the Cloud", death), func(status map[string]any) bool { return status["state"] == "off" })
	}
	wantRefusal(t, b.Get(listing, master), http.StatusServiceUnavailable, "cloud_crash_loop", "a listing after three deaths")
	device := theCloud(t, h)
	if got := h.Manager.Count("wake:" + device); got != 3 {
		t.Fatalf("the Cloud was woken %d times", got)
	}

	resetCloud(b, master, cloudReset)
	untilCloud(t, b, master, "the reset is ready", resetIs("ready"))
	b.Get(listing, master).Expect(http.StatusOK)
	b.Stop()
}

// Cost: one backend and a scripted manager, over a second: the first Cloud idles
// for a window before the second can start.
func TestCapacityCountsTheCloudsOfEveryUserAndACloudThatFindsNoneFailsToStart(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	h.Cloud().Capacity = backendtest.Ptr(uint64(1))
	idleAfter(h, 500*time.Millisecond)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	ana := b.CreateUser(master, "ana@example.test", "ana-pass-1", "user")
	b.CreateConversation(master, cloudFirst)
	b.CreateConversation(ana, cloudSecond)

	b.Get("/api/conversations/"+cloudFirst+"/fs", master).Expect(http.StatusOK)
	wantRefusal(t, b.Get("/api/conversations/"+cloudSecond+"/fs", ana), http.StatusServiceUnavailable, "cloud_capacity", "a Cloud beyond the capacity")
	wantRefusal(t, b.Post("/api/cloud/reset", ana, backendtest.Map{"operationId": cloudReset}), http.StatusConflict, "cloud_capacity", "a reset beyond the capacity")

	// Once the first Cloud stops, the second starts.
	untilCloud(t, b, master, "the first Cloud stops", func(status map[string]any) bool { return status["state"] == "off" })
	b.Get("/api/conversations/"+cloudSecond+"/fs", ana).Expect(http.StatusOK)
	if devices := h.Manager.Devices(); len(devices) != 2 {
		t.Fatalf("the manager made %v", devices)
	}
	b.Stop()
}

// Cost: one backend and a scripted manager, several seconds: the boot timeout of
// three seconds passes in real time.
func TestABootWhoseRunnerNeverConnectsFailsSavesWhatItStartedAndSaysWhy(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	// Short for the runner that never connects, and long enough for the one that
	// does on a busy machine.
	h.Cloud().RunnerConnectionMs = backendtest.Ptr(uint64(3000))
	b, master := h.StartSetUp()
	b.CreateConversation(master, cloudFirst)
	h.Manager.Configure(func(script *scripted.Script) { script.SilentWake = true })
	listing := "/api/conversations/" + cloudFirst + "/fs"
	refused := b.Get(listing, master)
	wantRefusal(t, refused, http.StatusServiceUnavailable, "cloud_unavailable", "a boot whose runner never connects")
	contains(t, refused.ErrorMessage(), "boot timeout")
	device := theCloud(t, h)
	if got := h.Manager.Count("hibernate:" + device); got != 1 {
		t.Fatalf("the manager was asked to hibernate %d times", got)
	}
	failed := cloudStatus(b, master)
	if failed["state"] != "off" {
		t.Fatalf("the Cloud is %v", failed["state"])
	}
	if message, _ := failed["error"].(string); !strings.Contains(message, "boot timeout") {
		t.Fatalf("the Cloud's error is %v", failed["error"])
	}

	h.Manager.Configure(func(script *scripted.Script) { script.SilentWake = false })
	b.Get(listing, master).Expect(http.StatusOK)
	if got := cloudStatus(b, master)["error"]; got != nil {
		t.Fatalf("the Cloud still has the error %v", got)
	}
	b.Stop()
}

// Cost: one backend started twice, a scripted manager and a Cloud's runner booted
// twice, several seconds: the Cloud installs the builtin package.
func TestTheCloudsFilesAndTodosAndTheUsageLedgerSurviveABackendRestart(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "/a")
	b.CreateConversation(master, cloudFirst)
	work := b.Open(master, vendor, cloudFirst, provider, "/a")
	script := "demi file create notes.md <<'EOF'\nkeep me\nEOF\ndemi todo add \"still here\""
	stored := work.Turn(cloudShell("t1", script), backendtest.Say("stored"))
	contains(t, stored.Result(t, 0), "Created notes.md")
	requests := func(session *backendtest.Session) float64 {
		total := 0.0
		for _, group := range usageTotals(b, session) {
			total += backendtest.At(group, "requests").(float64)
		}
		return total
	}
	before := requests(master)

	// The backend's close saves the Cloud; the next start boots nothing until a
	// command needs it. It starts at the address it had: the Cloud's runner keeps
	// its state, which names that address as its backend's.
	b.Stop()
	b = h.Start()
	work.Reconnect(b, master, cloudFirst, provider)
	found := work.Turn(cloudShell("t2", "cat notes.md && demi todo list"), backendtest.Say("found"))
	contains(t, found.Result(t, 0), "keep me", "still here")
	if after := requests(master); after != before+2 {
		t.Fatalf("the ledger counts %v requests, not %v", after, before+2)
	}
	device := theCloud(t, h)
	if got := h.Manager.Count("wake:" + device); got != 2 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, about a second,
// and 8 MiB written.
func TestShutdownEndsAnOpenDownloadSavesTheCloudAndReportsASaveThatFailed(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	b.CreateConversation(master, cloudFirst)
	b.Get("/api/conversations/"+cloudFirst+"/fs", master).Expect(http.StatusOK)
	device := theCloud(t, h)
	big := h.Manager.Home(device) + "/sessions/" + cloudFirst + "/big.bin"
	writeFile(t, big, strings.Repeat("\x07", 8<<20))
	// A download the browser stopped reading holds the Cloud.
	download, err := b.Send(backendtest.Request{Path: "/api/conversations/" + cloudFirst + "/fs/raw?" + query("path", big), Session: master})
	if err != nil {
		t.Fatal(err)
	}
	defer download.Body.Close()
	if download.StatusCode != http.StatusOK {
		t.Fatalf("the download answers %d", download.StatusCode)
	}

	failure := "the disk is full"
	h.Manager.Configure(func(script *scripted.Script) { script.FailHibernate = &failure })
	url := b.URL
	if code := b.Shutdown(); code == 0 {
		t.Fatal("a shutdown that failed to save the Cloud succeeded")
	}
	contains(t, b.Logs(), failure)
	// The download ended rather than kept the Cloud from its save.
	calls := h.Manager.Calls()
	saved := slices.Index(calls, "hibernate:"+device)
	last := -1
	for index, call := range calls {
		if call == "reconcile" {
			last = index
		}
	}
	if saved < 0 || saved > last {
		t.Fatalf("the Cloud was not saved before the last reconcile: %v", calls)
	}
	// The listener is gone.
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 5 * time.Second}
	if response, err := client.Get(url + "/api/setup"); err == nil {
		_ = response.Body.Close()
		t.Fatal("the backend still answers after shutdown")
	}
}

// Cost: one backend, a scripted manager and a paired device, several seconds:
// the Cloud boots, and three idle windows pass in real time: the conversation
// works past the window its first file read started, so that work that did not
// count would let the Cloud stop meanwhile, and then rests for one.
func TestAnAttachedCloudStaysAwakeWhileTheConversationWorksOnItsPairedTarget(t *testing.T) {
	t.Parallel()
	const window = 600 * time.Millisecond
	h := backendtest.New(t)
	idleAfter(h, window)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	paired := b.Pair(master, "paired")
	b.CreateConversation(master, cloudFirst)
	listing := "/api/conversations/" + cloudFirst + "/fs"
	b.Get(listing, master).Expect(http.StatusOK)
	device := theCloud(t, h)
	b.SwitchTo(master, cloudFirst, paired, paired.Home())

	// Work on the paired target, with the Cloud attached, keeps it awake past its
	// idle window: it stops a full window after the conversation rests, and not
	// before. The last activity ends after its request was sent, so the window is
	// counted from the sending.
	working := time.Now()
	rested := working
	for rested.Sub(working) < window*2 {
		rested = time.Now()
		b.Get(listing, master).Expect(http.StatusOK)
		time.Sleep(100 * time.Millisecond)
	}
	stopped := h.Manager.Arrival("hibernate:"+device, backendtest.Patience)
	if stopped.Before(rested.Add(window)) {
		t.Fatalf("the Cloud stopped %s after the work began and the work rested after %s", stopped.Sub(working), rested.Sub(working))
	}
	b.Stop()
}

// Cost: one backend started twice, killed once, a scripted manager and a Cloud's
// runner, about a second.
func TestABackendThatStoppedInTheMiddleOfAResetFinishesItsDiskStepWhenItStartsAndBootsNothing(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	b.CreateConversation(master, cloudFirst)
	b.Get("/api/conversations/"+cloudFirst+"/fs", master).Expect(http.StatusOK)
	device := theCloud(t, h)
	// The backend stops once the reset has selected its base and is rebuilding
	// the disks: the manager holds the reset at its disk step, and the backend is
	// killed there, as a crash would.
	hold := scripted.NewResetHold()
	h.Manager.Configure(func(script *scripted.Script) { script.HoldReset = hold })
	resetCloud(b, master, cloudReset)
	<-hold.Arrived()
	b.Kill()
	h.Manager.Configure(func(script *scripted.Script) { script.HoldReset = nil })
	hold.Proceed()
	contextBefore := func(b *backendtest.Backend) any { return b.Summary(master, cloudFirst)["contextVersion"] }

	b = h.Start()
	calls := h.Manager.Calls()
	started := -1
	for index, call := range calls {
		if call == "reconcile" {
			started = index
		}
	}
	want := fmt.Sprintf("reset:%s:%s:test-base", device, cloudReset)
	// The killed backend's reset call reached the manager once already, and the
	// recovery repeats it.
	if after := calls[started+1:]; !slices.Equal(after, []string{want}) {
		t.Fatalf("the manager was asked %v after the start", after)
	}
	recovered := cloudStatus(b, master)
	if recovered["state"] != "off" || backendtest.At(recovered, "operation.phase") != "failed" ||
		backendtest.At(recovered, "operation.error") != "Reset disks recovered; retry to start Cloud" {
		t.Fatalf("the recovered Cloud is %v", recovered)
	}
	announced := contextBefore(b)
	if got := h.Manager.Count("wake:" + device); got != 1 {
		t.Fatalf("the Cloud was woken %d times", got)
	}

	// A reset recorded as failed is not resumed when the backend starts again:
	// only a retry runs it.
	b.Stop()
	b = h.Start()
	calls = h.Manager.Calls()
	started = -1
	for index, call := range calls {
		if call == "reconcile" {
			started = index
		}
	}
	if after := calls[started+1:]; len(after) != 0 {
		t.Fatalf("the manager was asked %v after another start", after)
	}

	// A retry of the same reset starts the Cloud, and its announcement, made at
	// the recovery, advances the conversation's context no further.
	resetCloud(b, master, cloudReset)
	untilCloud(t, b, master, "the retried reset is ready", resetIs("ready"))
	if got := h.Manager.Count("wake:" + device); got != 2 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	if got := contextBefore(b); got != announced {
		t.Fatalf("the context moved from %v to %v", announced, got)
	}
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, several seconds:
// the Cloud boots and installs the builtin package, and its lifetime cap of two
// seconds passes in real time.
func TestAtItsLifetimeCapTheCloudEndsTheJobsNothingAttendsAndStops(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	h.Cloud().LifetimeCapMs = backendtest.Ptr(uint64(2000))
	h.Cloud().SweepMs = backendtest.Ptr(uint64(100))
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "/a")
	b.CreateConversation(master, cloudFirst)
	work := b.Open(master, vendor, cloudFirst, provider, "/a")
	// The model leaves a command running after its turn: it holds the Cloud, and
	// no idle stop would ever come.
	left := work.Turn(backendtest.ShellCall("t1", "sleep 30; echo late", time.Second), backendtest.Say("left it running"))
	if !strings.HasPrefix(left.Result(t, 0), "status: running") {
		t.Fatalf("the command is %s", left.Result(t, 0))
	}
	device := theCloud(t, h)
	backendtest.Eventually(t, "the Cloud stops at its lifetime cap", func() bool {
		return h.Manager.Count("hibernate:"+device) == 1 && !h.Manager.Running(device)
	})
	untilCloud(t, b, master, "the Cloud is off", func(status map[string]any) bool { return status["state"] == "off" })
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, several seconds:
// the Cloud boots and installs the builtin package, and three turns run a shell
// job each.
func TestCloudProjectsShareTheUsersOneMachineAndADeletedProjectKeepsItsFiles(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	b, master := h.StartSetUp()
	// Two projects made at once are directories of one Cloud, booted once.
	projects := make([]map[string]any, 2)
	backendtest.Concurrently(2, func(index int) {
		answer, err := b.TryDo(backendtest.Request{
			Method: http.MethodPost, Path: "/api/workspaces", Session: master,
			Body: backendtest.Map{"kind": "cloud", "name": []string{"first", "second"}[index]},
		})
		if err != nil || answer.Status != http.StatusCreated {
			t.Errorf("the project is %v %v", answer, err)
			return
		}
		projects[index], _ = answer.At("workspace").(map[string]any)
	})
	first, second := projects[0], projects[1]
	device := theCloud(t, h)
	if first["deviceId"] != device || second["deviceId"] != device {
		t.Fatalf("the projects are on %v and %v, not %s", first["deviceId"], second["deviceId"], device)
	}
	home := h.Manager.Home(device)
	if first["path"] != fmt.Sprintf("%s/projects/%s", home, first["id"]) || first["path"] == second["path"] {
		t.Fatalf("the projects are at %v and %v", first["path"], second["path"])
	}
	if info, err := os.Stat(second["path"].(string)); err != nil || !info.IsDir() {
		t.Fatalf("the second project's directory: %v", err)
	}
	if got := h.Manager.Count("wake:" + device); got != 1 {
		t.Fatalf("the Cloud was woken %d times", got)
	}

	firstModel := b.Anthropic(master, vendor, "/a")
	secondModel := b.Anthropic(master, vendor, "/b")
	for id, workspace := range map[string]map[string]any{cloudFirst: first, cloudSecond: second} {
		b.CreateConversation(master, id)
		b.Patch("/api/conversations/"+id, master, backendtest.Map{
			"target": backendtest.Map{"kind": "workspace", "workspaceId": workspace["id"]},
		}).Expect(http.StatusOK)
	}
	a := b.Open(master, vendor, cloudFirst, firstModel, "/a")
	second2 := b.Open(master, vendor, cloudSecond, secondModel, "/b")
	wrote := a.Turn(cloudShell("a1", "printf shared > note"), backendtest.Say("written"))
	contains(t, wrote.Result(t, 0), "exitCode: 0")
	read := fmt.Sprintf("cat '%s/note'", first["path"])
	seen := second2.Turn(cloudShell("b1", read), backendtest.Say("read"))
	contains(t, seen.Result(t, 0), "shared")

	// A project a conversation targets stays; once none does, it goes and its
	// files stay.
	wantRefusal(t, b.Delete("/api/workspaces/"+first["id"].(string), master), http.StatusConflict, "workspace_in_use", "deleting a targeted project")
	b.Patch("/api/conversations/"+cloudFirst, master, backendtest.Map{"target": backendtest.Map{"kind": "cloud"}}).Expect(http.StatusOK)
	b.Delete("/api/workspaces/"+first["id"].(string), master).Expect(http.StatusNoContent)
	kept := second2.Turn(cloudShell("b2", read), backendtest.Say("still there"))
	contains(t, kept.Result(t, 0), "shared")
	b.Stop()
}

// keepsNoJob waits until the Cloud's runner holds no job directory: it keeps
// nothing of a command once the backend has what it needs of it.
func keepsNoJob(t *testing.T, h *backendtest.Harness, device string) {
	t.Helper()
	jobs := filepath.Join(h.Manager.State(device), "jobs")
	backendtest.Eventually(t, "the Cloud keeps no job directory", func() bool {
		entries, _ := os.ReadDir(jobs)
		for _, entry := range entries {
			if entry.IsDir() {
				return false
			}
		}
		return true
	})
}

// twoTurns starts a turn of each conversation and waits for both, so that the
// two are under way at once.
func twoTurns(a, b *backendtest.Driven, aAnswers, bAnswers []*scripted.Response) (backendtest.Turn, backendtest.Turn) {
	beforeA := a.Start(aAnswers...)
	beforeB := b.Start(bAnswers...)
	a.Socket.UntilIdle()
	b.Socket.UntilIdle()
	return a.Observe(beforeA), b.Observe(beforeB)
}

// Cost: one backend, a scripted manager and a Cloud's runner booted twice, a few
// seconds: the Cloud boots for the first uses and installs the builtin package,
// stops after an idle window, and boots again.
func TestTheFirstUsesOfTwoConversationsBootOneCloudWhichStopsWhenIdleAndWakesForTheNextOperation(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	idleAfter(h, 600*time.Millisecond)
	// The conversations' own idle watches read their activity once an hour here,
	// so the Cloud's own idle watch is what stops it.
	h.Lifecycle().IdlePollMs = backendtest.Ptr(uint64(time.Hour.Milliseconds()))
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	firstModel := b.Anthropic(master, vendor, "/a")
	secondModel := b.Anthropic(master, vendor, "/b")
	for _, id := range []string{cloudFirst, cloudSecond} {
		b.CreateConversation(master, id)
	}
	// Neither history nor an open conversation needs the Cloud.
	a := b.Open(master, vendor, cloudFirst, firstModel, "/a")
	second := b.Open(master, vendor, cloudSecond, secondModel, "/b")
	if state := cloudStatus(b, master)["state"]; state != "unallocated" {
		t.Fatalf("the Cloud is %v", state)
	}
	if calls := h.Manager.Calls(); !slices.Equal(calls, []string{"reconcile"}) {
		t.Fatalf("the manager was called %v", calls)
	}

	// A lease of the first conversation's file gate is its work: the Cloud stays
	// awake until the test has looked, and idles once it ends.
	working := b.Control.EnterGate(master.User.ID, cloudFirst, "demand")
	first, other := twoTurns(a, second,
		[]*scripted.Response{cloudShell("a1", "echo 0 > note"), backendtest.Say("a wrote")},
		[]*scripted.Response{cloudShell("b1", "echo 1 > note"), backendtest.Say("b wrote")})
	contains(t, first.Result(t, 0), "exitCode: 0")
	contains(t, other.Result(t, 0), "exitCode: 0")
	device := theCloud(t, h)
	if got := h.Manager.Count("wake:" + device); got != 1 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	home := h.Manager.Home(device)
	if note := readFile(t, filepath.Join(home, "sessions", cloudFirst, "note")); note != "0\n" {
		t.Fatalf("the first note is %q", note)
	}
	if note := readFile(t, filepath.Join(home, "sessions", cloudSecond, "note")); note != "1\n" {
		t.Fatalf("the second note is %q", note)
	}
	keepsNoJob(t, h, device)
	running := cloudStatus(b, master)
	if running["state"] != "running" || backendtest.At(running, "device.id") != device {
		t.Fatalf("the Cloud is %v", running)
	}

	// The conversation's files are in its session directory on the Cloud.
	listed := b.Get("/api/conversations/"+cloudFirst+"/fs", master).Expect(http.StatusOK)
	if listed.Str("path") != filepath.Join(home, "sessions", cloudFirst) {
		t.Fatalf("the directory is %s", listed.Str("path"))
	}
	if !containsEntry(listed.At("entries"), "note") {
		t.Fatalf("the listing has no note: %s", listed.Body)
	}

	// Idle, the Cloud is saved and stops once.
	working.Release()
	backendtest.Eventually(t, "the idle Cloud stops", func() bool {
		return h.Manager.Count("hibernate:"+device) == 1 && !h.Manager.Running(device)
	})
	untilCloud(t, b, master, "the Cloud is off", func(status map[string]any) bool { return status["state"] == "off" })

	// A file read wakes it, and the next command runs on the awake Cloud.
	path := filepath.Join(home, "sessions", cloudFirst, "note")
	read := b.Get("/api/conversations/"+cloudFirst+"/fs/file?"+query("path", path), master).Expect(http.StatusOK)
	if read.Str("text") != "0\n" {
		t.Fatalf("the note reads %s", read.Body)
	}
	next := a.Turn(cloudShell("a2", "cat note"), backendtest.Say("awake"))
	contains(t, next.Result(t, 0), "0\n")
	if got := h.Manager.Count("wake:" + device); got != 2 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	b.Stop()
}

// containsEntry reports whether a directory listing's entries name name.
func containsEntry(entries any, name string) bool {
	list, _ := entries.([]any)
	for _, entry := range list {
		if backendtest.At(entry, "name") == name {
			return true
		}
	}
	return false
}

// Cost: one backend, a scripted manager and a Cloud's runner, about three
// seconds: the Cloud boots and installs the fixture package, and the first
// conversation's idle window passes in real time while the second works.
func TestAConversationThatIdlesOnACloudAnotherKeepsRunningHearsItsReleaseThere(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithNativeFixture())
	idleAfter(h, 600*time.Millisecond)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	firstModel := b.Anthropic(master, vendor, "/a")
	secondModel := b.Anthropic(master, vendor, "/b")
	for _, id := range []string{cloudFirst, cloudSecond} {
		b.CreateConversation(master, id)
	}
	a := b.Open(master, vendor, cloudFirst, firstModel, "/a")
	second := b.Open(master, vendor, cloudSecond, secondModel, "/b")
	// The second conversation works throughout, which keeps the Cloud running.
	working := b.Control.EnterGate(master.User.ID, cloudSecond, "demand")
	// Each runs a job, and the fixture service holds each conversation, as the
	// browser service holds a conversation's Chrome until its release.
	first, other := twoTurns(a, second,
		[]*scripted.Response{cloudShell("a1", "echo ran"), backendtest.Say("ran")},
		[]*scripted.Response{cloudShell("b1", "echo ran"), backendtest.Say("ran")})
	contains(t, first.Result(t, 0), "exitCode: 0")
	contains(t, other.Result(t, 0), "exitCode: 0")
	for _, id := range []string{cloudFirst, cloudSecond} {
		_, code, reason := b.OpenStream(master, id, "retain").Received()
		if code != 1000 || reason != "completed" {
			t.Fatalf("the stream closes with %d %s", code, reason)
		}
	}
	device := theCloud(t, h)

	// A window after its last activity, the first conversation hears its release
	// on the running Cloud: the service lets it go, and keeps the second.
	backendtest.Eventually(t, "the idle conversation's release reaches the service", func() bool {
		held, _, _ := b.OpenStream(master, cloudSecond, "held").Received()
		return string(backendtest.Marshal(backendtest.Decode(t, held))) == `{"conversations":["`+cloudSecond+`"]}`
	})
	// The machine runs on: it booted once and never stopped.
	if got := h.Manager.Count("wake:" + device); got != 1 {
		t.Fatalf("the Cloud was woken %d times", got)
	}
	if got := h.Manager.Count("hibernate:" + device); got != 0 {
		t.Fatalf("the Cloud was stopped %d times", got)
	}
	if state := cloudStatus(b, master)["state"]; state != "running" {
		t.Fatalf("the Cloud is %v", state)
	}
	working.Release()
	b.Stop()
}

// openHeldByAReset opens the conversation on a fresh socket while a reset holds
// its file gate, and answers the socket once the open went through: the open
// waits at the gate instead of failing, and finishes by itself when the reset
// ends.
func openHeldByAReset(t *testing.T, b *backendtest.Backend, master *backendtest.Session, hold *scripted.ResetHold, id string) *backendtest.Socket {
	t.Helper()
	socket := b.Connect(master, id)
	socket.Send(backendtest.Frame{"type": "open"})
	b.Control.WaitGateEntrants(master.User.ID, id, 1)
	hold.Proceed()
	handshake := socket.UntilType("pending_steers")
	if handshake[0]["type"] != "opened" {
		t.Fatalf("the open answers %v", handshake[0])
	}
	return socket
}

// Cost: one backend, a scripted manager, a Cloud's runner and a paired device,
// a few seconds: the Cloud installs the builtin package.
func TestAResetHoldsACloudConversationUntilItEndsAndLeavesOneThatOnlyHasTheCloudAttachedRunning(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	b, master := h.StartSetUp()
	alpha := b.Pair(master, "alpha")
	cloudModel := b.Anthropic(master, vendor, "/cloud")
	localModel := b.Anthropic(master, vendor, "/local")
	b.CreateConversation(master, cloudFirst)
	b.CreateConversation(master, cloudSecond)
	cloud := b.Open(master, vendor, cloudFirst, cloudModel, "/cloud")
	wrote := cloud.Turn(cloudShell("c1", "echo kept > note"), backendtest.Say("written"))
	contains(t, wrote.Result(t, 0), "exitCode: 0")
	device := theCloud(t, h)
	// The second conversation leaves the Cloud for alpha, which keeps the Cloud
	// attached.
	b.SwitchTo(master, cloudSecond, alpha, alpha.Home())
	hosts, _ := b.Get("/api/conversations/"+cloudSecond+"/hosts", master).At("hosts").([]any)
	if len(hosts) != 1 || backendtest.At(hosts[0], "deviceId") != device {
		t.Fatalf("the attached hosts are %v", hosts)
	}
	local := b.Open(master, vendor, cloudSecond, localModel, "/local")

	hold := scripted.NewResetHold()
	h.Manager.Configure(func(script *scripted.Script) { script.HoldReset = hold })
	resetCloud(b, master, cloudReset)
	<-hold.Arrived()

	// The conversation on alpha is opened, written to and answered.
	local.Reconnect(b, master, cloudSecond, localModel)
	during := local.Turn(cloudShell("l1", "echo on-alpha"), backendtest.Say("done"))
	contains(t, during.Result(t, 0), "on-alpha")

	// The Cloud conversation waits instead of failing, at its file gate, which
	// the reset holds, and opens by itself once the reset ends.
	cloud.Socket = openHeldByAReset(t, b, master, hold, cloudFirst)
	after := cloud.Turn(cloudShell("c2", "cat note"), backendtest.Say("read"))
	contains(t, after.Result(t, 0), "kept")
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, about a second.
func TestTheCloudsDeviceLogAnswersWhileItRunsAndAStoppedCloudSaysSoWithoutWaking(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	idleAfter(h, 400*time.Millisecond)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	b.CreateConversation(master, cloudFirst)
	// A lease of the conversation's file gate is its work: the Cloud stays awake
	// while the test reads its log, and idles once it ends.
	working := b.Control.EnterGate(master.User.ID, cloudFirst, "demand")
	listing := "/api/conversations/" + cloudFirst + "/fs"
	b.Get(listing, master).Expect(http.StatusOK)
	device := theCloud(t, h)
	log := "/api/devices/" + device + "/log"
	// The runner writes a line to its files a moment after it connected.
	onlines := func() int {
		read := b.Get(log, master)
		if read.Status != http.StatusOK {
			return 0
		}
		count := 0
		lines, _ := read.At("lines").([]any)
		for _, line := range lines {
			if backendtest.At(line, "text") == "online" {
				count++
			}
		}
		return count
	}
	backendtest.Eventually(t, "the running Cloud's log says it is online", func() bool { return onlines() == 1 })

	working.Release()
	untilCloud(t, b, master, "the idle Cloud stops", func(status map[string]any) bool { return status["state"] == "off" })
	wantRefusal(t, b.Get(log, master), http.StatusConflict, "device_offline", "the log of a stopped Cloud")
	if got := h.Manager.Count("wake:" + device); got != 1 {
		t.Fatalf("the Cloud was woken %d times", got)
	}

	// Woken, it answers again, with what it wrote before it stopped.
	b.Get(listing, master).Expect(http.StatusOK)
	backendtest.Eventually(t, "the woken Cloud's log keeps the earlier boot", func() bool { return onlines() == 2 })
	b.Stop()
}

// Cost: one backend, a scripted manager, a Cloud's runner and a paired device,
// about two seconds: the conversation works on the device, and only its provider,
// the claude-code family's process, uses the Cloud.
func TestAResetHoldsAConversationOnAPairedDeviceWhoseProviderRunsOnTheCloud(t *testing.T) {
	t.Parallel()
	catalog := scripted.StartVendor(t)
	catalog.RespondAt("/api.json", claudeModels())
	h := backendtest.New(t, backendtest.WithModelsDev(catalog.URL("/api.json")))
	b, master := h.StartSetUp()
	alpha := b.Pair(master, "alpha")
	provider := b.Post("/api/providers/setup-token", master, backendtest.Map{"token": claudeFirstToken, "label": "Claude"}).Expect(http.StatusCreated).Str("provider.id")
	// Its files and commands are on alpha; only its provider uses the Cloud.
	b.CreateConversation(master, cloudFirst)
	b.SwitchTo(master, cloudFirst, alpha, alpha.Home())
	b.Choose(master, cloudFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, cloudFirst)
	socket.Open()
	socket.Close()

	hold := scripted.NewResetHold()
	h.Manager.Configure(func(script *scripted.Script) { script.HoldReset = hold })
	resetCloud(b, master, cloudReset)
	<-hold.Arrived()
	// It waits instead of failing, at its file gate, which the reset holds, and
	// opens by itself when the reset ends.
	openHeldByAReset(t, b, master, hold, cloudFirst)
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, about two seconds:
// the idle window passes in real time, twice while the lease is held and once
// after it ends.
func TestALeaseOfAConversationsFileGateKeepsTheCloudAwakeAcrossAnIdleWindow(t *testing.T) {
	t.Parallel()
	const window = 400 * time.Millisecond
	h := backendtest.New(t)
	idleAfter(h, window)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	b.CreateConversation(master, cloudFirst)
	// Reading the conversation's files wakes the Cloud it works on.
	b.Get("/api/conversations/"+cloudFirst+"/fs", master).Expect(http.StatusOK)
	device := theCloud(t, h)

	// A lease of the conversation's file gate is its work: over two idle windows
	// the Cloud is not stopped.
	lease := b.Control.EnterGate(master.User.ID, cloudFirst, "demand")
	time.Sleep(2 * window)
	if got := h.Manager.Count("hibernate:" + device); got != 0 {
		t.Fatalf("the Cloud was stopped %d times while a lease held its conversation's gate", got)
	}
	if state := cloudStatus(b, master)["state"]; state != "running" {
		t.Fatalf("the Cloud is %v", state)
	}

	// Released, the Cloud stops a full window after the lease ended, and not
	// before.
	released := time.Now()
	lease.Release()
	stopped := h.Manager.Arrival("hibernate:"+device, backendtest.Patience)
	if stopped.Before(released.Add(window)) {
		t.Fatalf("the Cloud stopped %v after the lease ended", stopped.Sub(released))
	}
	b.Stop()
}
