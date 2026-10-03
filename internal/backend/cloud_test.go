package backend_test

import (
	"bytes"
	"context"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

const cloudFirst = "4e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01"
const cloudSecond = "4e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02"
const cloudReset = "6e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a09"

func (s *hostScenario) cloudStatus() webapi.CloudStatus {
	s.t.Helper()
	a := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/cloud", "", 200)
	state, err := webapi.DecodeCloudStatus(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return state
}

// cloudUntil reads the same status route as Rust, woken by product state changes.
func (s *hostScenario) cloudUntil(check func(webapi.CloudStatus) bool) webapi.CloudStatus {
	s.t.Helper()
	changes := s.b.Backend.Services().Sync.Register(s.user.User.ID, database.TokenHash{})
	defer changes.Release()
	for {
		changes.Take()
		status := s.cloudStatus()
		if check(status) {
			return status
		}
		if err := changes.Marked(s.ctx); err != nil {
			s.t.Fatal(err)
		}
	}
}
func (s *hostScenario) theCloud() webapi.DeviceID {
	s.t.Helper()
	devices := s.manager.Devices()
	if len(devices) != 1 {
		s.t.Fatalf("Cloud devices: %v", devices)
	}
	return devices[0]
}
func (s *hostScenario) resetCloud(id string) webapi.CloudResetAnswer {
	s.t.Helper()
	a := conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/cloud/reset", fmt.Sprintf(`{"operationId":%q}`, id), 202)
	answer, err := webapi.DecodeCloudResetAnswer(a.Body)
	if err != nil {
		s.t.Fatal(err)
	}
	return answer
}

// Three real runner boots and a reset exercise crash-loop admission across the API and manager wire.
func TestCloudCrashLoopRequiresReset(t *testing.T) {
	s := newHostScenario(t, "")
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	listing := "/api/conversations/" + cloudFirst + "/fs"
	for death := 0; death < 3; death++ {
		conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 200)
		if err := s.manager.Kill(s.ctx, s.theCloud()); err != nil {
			t.Fatal(err)
		}
		s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 503), webapi.ErrorCodeCloudCrashLoop)
	device := s.theCloud()
	if s.manager.Count("wake:"+string(device)) != 3 {
		t.Fatal(s.manager.Calls())
	}
	s.resetCloud(cloudReset)
	s.cloudUntil(func(status webapi.CloudStatus) bool {
		return status.Operation != nil && status.Operation.Phase == webapi.ResetPhaseReady
	})
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 200)
}

// Capacity is released by the real idle watch; the scenario waits on its page event.
func TestCloudCapacityIsSharedAcrossUsers(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Cloud.Capacity = 1
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	h.Config.Lifecycle.IdleWindow = 500 * time.Millisecond
	h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	b, master, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, master, manager}
	if err := h.AddUser(s.ctx, "ana@example.test", "ana-pass-1", webapi.RoleUser); err != nil {
		t.Fatal(err)
	}
	ana, err := b.Login(s.ctx, "ana@example.test", "ana-pass-1")
	if err != nil {
		t.Fatal(err)
	}
	other := *s
	other.user = ana
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	conversationCreate(other.ctx, other.t, other.b, &other.user, cloudSecond)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+cloudFirst+"/fs", "", 200)
	conversationRefusal(other.t, conversationRequest(other.ctx, other.t, other.b, &other.user, "GET", "/api/conversations/"+cloudSecond+"/fs", "", 503), webapi.ErrorCodeCloudCapacity)
	conversationRefusal(other.t, conversationRequest(other.ctx, other.t, other.b, &other.user, "POST", "/api/cloud/reset", fmt.Sprintf(`{"operationId":%q}`, cloudReset), 409), webapi.ErrorCodeCloudCapacity)
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	conversationRequest(other.ctx, other.t, other.b, &other.user, "GET", "/api/conversations/"+cloudSecond+"/fs", "", 200)
	if len(manager.Devices()) != 2 {
		t.Fatal(manager.Devices())
	}
}

// The three-second boot deadline is the behavior under test; no sleep polls it.
func TestCloudBootTimeoutSavesAndReportsFailure(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Cloud.RunnerConnection = 3 * time.Second
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	manager.SetScript(backendtest.MachineScript{SilentWake: true})
	listing := "/api/conversations/" + cloudFirst + "/fs"
	answer := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 503)
	failure, err := answer.ErrorBody()
	if err != nil {
		t.Fatal(err)
	}
	if failure.Code != webapi.ErrorCodeCloudUnavailable || !strings.Contains(failure.Message, "boot timeout") {
		t.Fatal(failure)
	}
	device := s.theCloud()
	if manager.Count("hibernate:"+string(device)) != 1 {
		t.Fatal(manager.Calls())
	}
	failed := s.cloudStatus()
	if failed.State != webapi.CloudStateOff || failed.Error == nil || !strings.Contains(*failed.Error, "boot timeout") {
		t.Fatal(failed)
	}
	manager.SetScript(backendtest.MachineScript{})
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 200)
	if s.cloudStatus().Error != nil {
		t.Fatal("boot error remains")
	}
}

func TestStartupRecoversResetDisksWithoutBooting(t *testing.T) {
	s := newHostScenario(t, "")
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+cloudFirst+"/fs", "", 200)
	device := s.theCloud()
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
	db, err := s.h.ControlDatabase(s.ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(s.ctx, `INSERT INTO managed_operations (device_id, operation_id, base_version, phase, error, updated_at) VALUES (?, ?, 'test-base', 'rebuilding', NULL, 0)`, device, cloudReset); err != nil {
		t.Fatal(err)
	}
	var before int64
	if err := db.QueryRowContext(s.ctx, "SELECT context_version FROM conversations WHERE id = ?", cloudFirst).Scan(&before); err != nil {
		t.Fatal(err)
	}
	s.b, err = s.h.Start(s.ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	calls := s.manager.Calls()
	last := -1
	for i, call := range calls {
		if call == "reconcile" {
			last = i
		}
	}
	want := fmt.Sprintf("reset:%s:%s:test-base", device, cloudReset)
	if last < 0 || len(calls[last+1:]) != 1 || calls[last+1] != want {
		t.Fatal(calls)
	}
	recovered := s.cloudStatus()
	if recovered.State != webapi.CloudStateOff || recovered.Operation == nil || recovered.Operation.Phase != webapi.ResetPhaseFailed || recovered.Operation.Error == nil || *recovered.Operation.Error != "Reset disks recovered; retry to start Cloud" {
		t.Fatalf("recovered: %+v", recovered)
	}
	contextVersion := func() int64 {
		var version int64
		if err := db.QueryRowContext(s.ctx, "SELECT context_version FROM conversations WHERE id = ?", cloudFirst).Scan(&version); err != nil {
			t.Fatal(err)
		}
		return version
	}
	if contextVersion() != before+1 {
		t.Fatal("recovery did not advance context")
	}
	if s.manager.Count("wake:"+string(device)) != 1 {
		t.Fatal(s.manager.Calls())
	}
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
	s.b, err = s.h.Start(s.ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	calls = s.manager.Calls()
	if calls[len(calls)-1] != "reconcile" {
		t.Fatal(calls)
	}
	s.resetCloud(cloudReset)
	s.cloudUntil(func(status webapi.CloudStatus) bool {
		return status.Operation != nil && status.Operation.Phase == webapi.ResetPhaseReady
	})
	if s.manager.Count("wake:"+string(device)) != 2 {
		t.Fatal(s.manager.Calls())
	}
	if contextVersion() != before+1 {
		t.Fatal("retry advanced context again")
	}
}

// cloudWork drives a conversation with the same scripted Anthropic endpoint as Rust.
type cloudWork struct {
	s            *hostScenario
	vendor       *providertest.MockVendor
	socket       *backendtest.ConversationSocket
	sent         int
	provider     string
	id           string
	firstRequest string
}

func (s *hostScenario) work(id string) *cloudWork {
	s.t.Helper()
	vendor := providertest.StartVendor(s.t)
	provider := conversationAnthropic(s.ctx, s.t, s.b, &s.user, vendor)
	conversationCreate(s.ctx, s.t, s.b, &s.user, id)
	w := &cloudWork{s: s, vendor: vendor, provider: provider, id: id}
	w.open()
	return w
}

// open reconnects this Cloud workload with its persisted provider selection.
func (w *cloudWork) open() {
	w.s.t.Helper()
	if w.socket != nil {
		wireMust(w.s.t, w.socket.Close(w.s.ctx))
	}
	conversationChoose(w.s.ctx, w.s.t, w.s.b, &w.s.user, w.id, w.provider, "claude-opus-4-8")
	w.socket = conversationOpen(w.s.ctx, w.s.t, w.s.b, &w.s.user, w.id)
}

func (w *cloudWork) turn(id, script, text string, timeout ...int) string {
	w.s.t.Helper()
	before := len(w.vendor.Requests())
	if script != "" {
		milliseconds := 20000
		if len(timeout) > 0 {
			milliseconds = timeout[0]
		}
		w.vendor.Respond(conversationShell(w.s.t, id, script, milliseconds))
	}
	w.vendor.Respond(conversationAnswer(w.s.t, []string{text}, 1, 1))
	w.sent++
	_, err := w.socket.Chat(w.s.ctx, fmt.Sprintf("m%d", w.sent), "go")
	wireMust(w.s.t, err)
	requests := w.vendor.Requests()[before:]
	if len(requests) == 0 {
		w.s.t.Fatal("no vendor request")
	}
	fields, err := contract.Object(requests[0].Body)
	wireMust(w.s.t, err)
	w.firstRequest = string(fields["messages"])
	if script == "" {
		return ""
	}
	return conversationToolResult(w.s.t, requests[len(requests)-1], id)
}

func TestArchivingStoppedCloudDoesNotWakeIt(t *testing.T) {
	s := newHostScenario(t, "")
	w := s.work(cloudFirst)
	if result := w.turn("t1", "echo ran", "ran"); !strings.Contains(result, "exitCode: 0") {
		t.Fatal(result)
	}
	device := s.theCloud()
	address := s.b.Address()
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
	var err error
	s.b, err = s.h.StartAt(s.ctx, t, address)
	if err != nil {
		t.Fatal(err)
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+cloudFirst, `{"archived":true}`, 200)
	if s.manager.Count("wake:"+string(device)) != 1 {
		t.Fatal(s.manager.Calls())
	}
	if s.cloudStatus().State != webapi.CloudStateOff {
		t.Fatal("Cloud woke")
	}
}

func TestCloudResetKeepsHomeIdentityAndAnnouncesOnce(t *testing.T) {
	s := newHostScenario(t, "demi-file")
	w := s.work(cloudFirst)
	if result := w.turn("t1", "echo retained > note", "written"); !strings.Contains(result, "exitCode: 0") {
		t.Fatal(result)
	}
	before := s.cloudStatus().Device
	if before == nil {
		t.Fatal("the Cloud was not made")
	}
	var answers [3]webapi.CloudResetAnswer
	var workers sync.WaitGroup
	for i := range answers {
		workers.Go(func() { answers[i] = s.resetCloud(cloudReset) })
	}
	workers.Wait()
	for _, answer := range answers {
		if string(answer.Operation.ID) != cloudReset {
			t.Fatal(answer)
		}
	}
	ready := s.cloudUntil(func(status webapi.CloudStatus) bool {
		return status.Operation != nil && status.Operation.Phase == webapi.ResetPhaseReady
	})
	if ready.State != webapi.CloudStateRunning || !reflect.DeepEqual(ready.Device, before) {
		t.Fatal(ready)
	}
	next := w.turn("t2", "cat note", "read")
	if !strings.Contains(next, "retained") || !strings.Contains(w.firstRequest, "[Cloud reset "+cloudReset+"]") {
		t.Fatal(next)
	}
	if s.resetCloud(cloudReset).Operation.Phase != webapi.ResetPhaseReady {
		t.Fatal("repeat reset not ready")
	}
	device := s.theCloud()
	resets := 0
	for _, call := range s.manager.Calls() {
		if strings.HasPrefix(call, "reset:") {
			resets++
		}
	}
	if resets != 1 || s.manager.Count(fmt.Sprintf("reset:%s:%s:test-base", device, cloudReset)) != 1 {
		t.Fatal(s.manager.Calls())
	}
	held := s.manager.HoldReset(t)
	const second = "7e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a0a"
	s.resetCloud(second)
	if err := held.UntilArrived(s.ctx, 1); err != nil {
		t.Fatal(err)
	}
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/cloud/reset", `{"operationId":"8e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a0b"}`, 409), webapi.ErrorCodeCloudResetting)
	held.Release()
	s.cloudUntil(func(status webapi.CloudStatus) bool {
		return status.Operation != nil && string(status.Operation.ID) == second && status.Operation.Phase == webapi.ResetPhaseReady
	})
}

func TestFailedCloudResetResumesOnSameBaseAndKeepsHome(t *testing.T) {
	s := newHostScenario(t, "demi-file")
	w := s.work(cloudFirst)
	if result := w.turn("t1", "printf retained > note", "written"); !strings.Contains(result, "exitCode: 0") {
		t.Fatal(result)
	}
	device := s.theCloud()
	failure := "image publication unavailable"
	s.manager.SetScript(backendtest.MachineScript{FailReset: &failure})
	s.resetCloud(cloudReset)
	failed := s.cloudUntil(func(status webapi.CloudStatus) bool {
		return status.Operation != nil && status.Operation.Phase == webapi.ResetPhaseFailed
	})
	if failed.State != webapi.CloudStateOff || failed.Operation.Error == nil || !strings.Contains(*failed.Operation.Error, failure) {
		t.Fatal(failed)
	}
	s.resetCloud(cloudReset)
	s.cloudUntil(func(status webapi.CloudStatus) bool {
		return status.Operation != nil && status.Operation.Phase == webapi.ResetPhaseReady
	})
	path := s.manager.Home(device) + "/sessions/" + cloudFirst + "/note"
	read := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+cloudFirst+"/fs/file?path="+url.QueryEscape(path), "", 200)
	file, err := webapi.DecodeFileText(read.Body)
	if err != nil {
		t.Fatal(err)
	}
	if file.Text != "retained" {
		t.Fatal(file.Text)
	}
	var resets []string
	for _, call := range s.manager.Calls() {
		if strings.HasPrefix(call, "reset:") {
			resets = append(resets, call)
		}
	}
	want := fmt.Sprintf("reset:%s:%s:test-base", device, cloudReset)
	if len(resets) != 2 || resets[0] != want || resets[1] != want {
		t.Fatal(resets)
	}
}

func TestQuietlyStoppedCloudRecoversOnceForConcurrentReads(t *testing.T) {
	s := newHostScenario(t, "demi-file")
	w := s.work(cloudFirst)
	if result := w.turn("t1", "echo retained > note", "saved"); !strings.Contains(result, "exitCode: 0") {
		t.Fatal(result)
	}
	device := s.theCloud()
	if err := s.manager.StopQuietly(s.ctx, device); err != nil {
		t.Fatal(err)
	}
	if err := s.b.UntilOnline(s.ctx, &s.user, device, false); err != nil {
		t.Fatal(err)
	}
	if s.cloudStatus().State != webapi.CloudStateRunning {
		t.Fatal("quiet stop changed logical status")
	}
	path := s.manager.Home(device) + "/sessions/" + cloudFirst + "/note"
	var workers sync.WaitGroup
	for range 2 {
		workers.Go(func() {
			read := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+cloudFirst+"/fs/file?path="+url.QueryEscape(path), "", 200)
			file, err := webapi.DecodeFileText(read.Body)
			if err != nil {
				t.Error(err)
				return
			}
			if file.Text != "retained\n" {
				t.Error(file.Text)
			}
		})
	}
	workers.Wait()
	if s.manager.Count("wake:"+string(device)) != 2 {
		t.Fatal(s.manager.Calls())
	}
}

func TestShutdownCancelsCloudBootAndSavesOnce(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	built, err := backendtest.BuildPackage(t.Context(), t, "demi-claude-code")
	if err != nil {
		t.Fatal(err)
	}
	if err := h.UsePackage(t.Context(), built); err != nil {
		t.Fatal(err)
	}
	manager.SetScript(backendtest.MachineScript{SilentWake: true})
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/providers/setup-token", `{"token":"sk-ant-oat01-shutdown-account","label":"Claude"}`, 201)
	booting := s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateBooting })
	if booting.Device == nil {
		t.Fatal("booting Cloud has no identity")
	}
	device := booting.Device.ID
	if _, err := manager.Arrival(s.ctx, "wake:"+string(device)); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(s.ctx, 10*time.Second)
	defer cancel()
	if err := b.Close(ctx); err != nil {
		t.Fatal(err)
	}
	if manager.Count("hibernate:"+string(device)) != 1 {
		t.Fatal(manager.Calls())
	}
}

func TestShutdownCutsCloudDownloadAndReportsFailedSave(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	b, closeReporting, err := backendtest.HostsReportingStart(t.Context(), t, h)
	if err != nil {
		t.Fatal(err)
	}
	user, err := b.Setup(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+cloudFirst+"/fs", "", 200)
	device := s.theCloud()
	big := s.manager.Home(device) + "/sessions/" + cloudFirst + "/big.bin"
	if err := os.WriteFile(big, bytes.Repeat([]byte{7}, 8<<20), 0644); err != nil {
		t.Fatal(err)
	}
	download, err := s.b.Response(s.ctx, "GET", "/api/conversations/"+cloudFirst+"/fs/raw?path="+url.QueryEscape(big), &s.user, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := download.Body.Close(); err != nil {
			t.Error(err)
		}
	}()
	if download.StatusCode != 200 {
		t.Fatal(download.Status)
	}
	failure := "the disk is full"
	s.manager.SetScript(backendtest.MachineScript{FailHibernate: &failure})
	err = closeReporting(s.ctx)
	if err == nil || !strings.Contains(err.Error(), "a Cloud was not saved") || !strings.Contains(err.Error(), failure) {
		t.Fatalf("shutdown: %v", err)
	}
	calls := s.manager.Calls()
	saved, reconciled := -1, -1
	for i, call := range calls {
		if call == "hibernate:"+string(device) {
			saved = i
		}
		if call == "reconcile" {
			reconciled = i
		}
	}
	if saved < 0 || saved >= reconciled {
		t.Fatal(calls)
	}
	if _, err := s.b.Read(s.ctx, "/api/setup", nil); err == nil {
		t.Fatal("listener still serves")
	}
}

func TestCloudFilesTodosAndUsageSurviveBackendRestart(t *testing.T) {
	s := newHostScenario(t, "demi-file")
	w := s.work(cloudFirst)
	script := "demi file create notes.md <<'EOF'\nkeep me\nEOF\ndemi todo add \"still here\""
	if result := w.turn("t1", script, "stored"); !strings.Contains(result, "Created notes.md") {
		t.Fatal(result)
	}
	requests := func() uint64 {
		read := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/usage", "", 200)
		usage, err := webapi.DecodeUsageTotals(read.Body)
		if err != nil {
			t.Fatal(err)
		}
		var total uint64
		for _, group := range usage.Totals {
			total += group.Requests
		}
		return total
	}
	before := requests()
	address := s.b.Address()
	if err := s.b.Close(s.ctx); err != nil {
		t.Fatal(err)
	}
	var err error
	s.b, err = s.h.StartAt(s.ctx, t, address)
	if err != nil {
		t.Fatal(err)
	}
	w.open()
	found := w.turn("t2", "cat notes.md && demi todo list", "found")
	if !strings.Contains(found, "keep me") || !strings.Contains(found, "still here") {
		t.Fatal(found)
	}
	if got := requests(); got != before+2 {
		t.Fatalf("requests %d, want %d", got, before+2)
	}
	if s.manager.Count("wake:"+string(s.theCloud())) != 2 {
		t.Fatal(s.manager.Calls())
	}
}

func TestCloudProjectsShareMachineAndDeletionKeepsFiles(t *testing.T) {
	s := newHostScenario(t, "demi-file")
	var projects [2]webapi.WorkspaceDTO
	var workers sync.WaitGroup
	for i, name := range []string{"first", "second"} {
		workers.Go(func() {
			answer := conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/workspaces", fmt.Sprintf(`{"kind":"cloud","name":%q}`, name), 201)
			created, err := webapi.DecodeWorkspaceAnswer(answer.Body)
			if err != nil {
				t.Error(err)
				return
			}
			projects[i] = created.Workspace
		})
	}
	workers.Wait()
	first, second := projects[0], projects[1]
	device := s.theCloud()
	if first.DeviceID != device || second.DeviceID != device {
		t.Fatal(projects)
	}
	if first.Path != s.manager.Home(device)+"/projects/"+string(first.ID) || first.Path == second.Path {
		t.Fatal(projects)
	}
	info, err := os.Stat(second.Path)
	if err != nil || !info.IsDir() {
		t.Fatalf("second project: %v", err)
	}
	if s.manager.Count("wake:"+string(device)) != 1 {
		t.Fatal(s.manager.Calls())
	}
	a, b := s.work(cloudFirst), s.work(cloudSecond)
	for i, id := range []string{cloudFirst, cloudSecond} {
		conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+id, fmt.Sprintf(`{"target":{"kind":"workspace","workspaceId":%q}}`, projects[i].ID), 200)
	}
	if result := a.turn("a1", "printf shared > note", "written"); !strings.Contains(result, "exitCode: 0") {
		t.Fatal(result)
	}
	read := "cat '" + first.Path + "/note'"
	if result := b.turn("b1", read, "read"); !strings.Contains(result, "shared") {
		t.Fatal(result)
	}
	route := "/api/workspaces/" + string(first.ID)
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", route, "", 409), webapi.ErrorCodeWorkspaceInUse)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+cloudFirst, `{"target":{"kind":"cloud"}}`, 200)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "DELETE", route, "", 204)
	if result := b.turn("b2", read, "still there"); !strings.Contains(result, "shared") {
		t.Fatal(result)
	}
}

// Two concurrent turns, an idle stop, and a second boot exercise shared Cloud ownership.
func TestConcurrentCloudUsesBootOnceAndIdleStopWakesOnDemand(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	built, err := backendtest.BuildPackage(t.Context(), t, "demi-file")
	if err != nil {
		t.Fatal(err)
	}
	if err := h.UsePackage(t.Context(), built); err != nil {
		t.Fatal(err)
	}
	h.Config.Lifecycle.IdleWindow = 600 * time.Millisecond
	h.Config.Lifecycle.IdlePoll = time.Hour
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	a, other := s.work(cloudFirst), s.work(cloudSecond)
	if s.cloudStatus().State != webapi.CloudStateUnallocated {
		t.Fatal("open allocated Cloud")
	}
	if calls := manager.Calls(); len(calls) != 1 || calls[0] != "reconcile" {
		t.Fatal(calls)
	}
	gate, err := backendtest.FileGate(s.ctx, b.Backend, user.User.ID, webapi.ConversationID(cloudFirst))
	if err != nil {
		t.Fatal(err)
	}
	working, err := gate.Enter(s.ctx, gates.Demand)
	if err != nil {
		t.Fatal(err)
	}
	defer working.Release()
	var results [2]string
	var workers sync.WaitGroup
	workers.Go(func() { results[0] = a.turn("a1", "echo 0 > note", "a wrote") })
	workers.Go(func() { results[1] = other.turn("b1", "echo 1 > note", "b wrote") })
	workers.Wait()
	for _, result := range results {
		if !strings.Contains(result, "exitCode: 0") {
			t.Fatal(result)
		}
	}
	device := s.theCloud()
	if manager.Count("wake:"+string(device)) != 1 {
		t.Fatal(manager.Calls())
	}
	home := manager.Home(device)
	for i, id := range []string{cloudFirst, cloudSecond} {
		data, err := os.ReadFile(home + "/sessions/" + id + "/note")
		if err != nil {
			t.Fatal(err)
		}
		if string(data) != fmt.Sprintf("%d\n", i) {
			t.Fatal(string(data))
		}
	}
	if err := backendtest.WaitRunnerJobsRemoved(s.ctx, manager.State(device)); err != nil {
		t.Fatal(err)
	}
	running := s.cloudStatus()
	if running.State != webapi.CloudStateRunning || running.Device == nil || running.Device.ID != device {
		t.Fatal(running)
	}
	listed := s.directoryListing("/api/conversations/" + cloudFirst + "/fs")
	if listed.Path != home+"/sessions/"+cloudFirst {
		t.Fatal(listed)
	}
	found := false
	for _, entry := range listed.Entries {
		if entry.Name == "note" {
			found = true
		}
	}
	if !found {
		t.Fatal(listed)
	}
	working.Release()
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	if manager.Count("hibernate:"+string(device)) != 1 || manager.Running(device) {
		t.Fatal(manager.Calls())
	}
	read := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+cloudFirst+"/fs/file?path="+url.QueryEscape(home+"/sessions/"+cloudFirst+"/note"), "", 200)
	file, err := webapi.DecodeFileText(read.Body)
	if err != nil {
		t.Fatal(err)
	}
	if file.Text != "0\n" {
		t.Fatal(file.Text)
	}
	if result := a.turn("a2", "cat note", "awake"); !strings.Contains(result, "0\n") {
		t.Fatal(result)
	}
	if manager.Count("wake:"+string(device)) != 2 {
		t.Fatal(manager.Calls())
	}
}

// The lifetime cap is two seconds; the abandoned job would otherwise hold the Cloud for thirty.
func TestCloudLifetimeCapEndsUnattendedJobs(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	built, err := backendtest.BuildPackage(t.Context(), t, "demi-file")
	if err != nil {
		t.Fatal(err)
	}
	if err := h.UsePackage(t.Context(), built); err != nil {
		t.Fatal(err)
	}
	h.Config.Cloud.LifetimeCap = 2 * time.Second
	h.Config.Cloud.Sweep = 100 * time.Millisecond
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	w := s.work(cloudFirst)
	result := w.turn("t1", "sleep 30; echo late", "left it running", 1000)
	if !strings.HasPrefix(result, "status: running") {
		t.Fatal(result)
	}
	device := s.theCloud()
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	if manager.Count("hibernate:"+string(device)) != 1 || manager.Running(device) {
		t.Fatal(manager.Calls())
	}
}

func TestCloudLogSurvivesIdleStopWithoutWakingOnRead(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Lifecycle.IdleWindow = 400 * time.Millisecond
	h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	gate, err := backendtest.FileGate(s.ctx, b.Backend, user.User.ID, webapi.ConversationID(cloudFirst))
	if err != nil {
		t.Fatal(err)
	}
	working, err := gate.Enter(s.ctx, gates.Demand)
	if err != nil {
		t.Fatal(err)
	}
	defer working.Release()
	listing := "/api/conversations/" + cloudFirst + "/fs"
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 200)
	device := s.theCloud()
	path := "/api/devices/" + string(device) + "/log"
	onlineCount := func() int {
		count := 0
		for _, line := range s.deviceLog(path).Lines {
			if line.Text == "online" {
				count++
			}
		}
		return count
	}
	if count := onlineCount(); count != 1 {
		t.Fatalf("online lines %d", count)
	}
	working.Release()
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	conversationRefusal(s.t, conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", path, "", 409), webapi.ErrorCodeDeviceOffline)
	if manager.Count("wake:"+string(device)) != 1 {
		t.Fatal(manager.Calls())
	}
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 200)
	if count := onlineCount(); count != 2 {
		t.Fatalf("online lines %d", count)
	}
}

// Several seconds: two real runners install packages and Cloud wakes three times.
func TestAttachedCloudWakesForBrowseAndCommands(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	built, err := backendtest.BuildPackage(t.Context(), t, "demi-file")
	if err != nil {
		t.Fatal(err)
	}
	if err := h.UsePackage(t.Context(), built); err != nil {
		t.Fatal(err)
	}
	h.Config.Lifecycle.IdleWindow = 800 * time.Millisecond
	h.Config.Lifecycle.IdlePoll = time.Hour
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	alpha := s.pair("alpha")
	if err := os.WriteFile(filepath.Join(alpha.Runner.Home(), "notes.txt"), []byte("on alpha\n"), 0644); err != nil {
		t.Fatal(err)
	}
	w := s.work(cloudFirst)
	route := "/api/conversations/" + cloudFirst
	conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", route+"/hosts", fmt.Sprintf(`{"deviceId":%q}`, alpha.ID()), 201)
	result := w.turn("t1", "echo report > report.txt && demi host shell --host alpha 'cat notes.txt'", "reached alpha", 30000)
	if !strings.Contains(result, "on alpha") {
		t.Fatal(result)
	}
	device := s.theCloud()
	session := manager.Home(device) + "/sessions/" + cloudFirst
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", route, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, alpha.ID(), alpha.Runner.Home()), 200)
	read := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", route+"/hosts", "", 200)
	hosts, err := webapi.DecodeAttachedHosts(read.Body)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, host := range hosts.Hosts {
		if host.DeviceID == device {
			found = true
			if host.Name != "Cloud" || host.Cwd == nil || *host.Cwd != session {
				t.Fatal(host)
			}
		}
	}
	if !found {
		t.Fatal("Cloud not attached")
	}
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	if manager.Running(device) {
		t.Fatal("idle Cloud runner still running")
	}
	listed := s.directoryListing(route + "/hosts/" + string(device) + "/fs?path=" + url.QueryEscape(session))
	found = false
	for _, entry := range listed.Entries {
		if entry.Name == "report.txt" {
			found = true
		}
	}
	if !found {
		t.Fatal(listed)
	}
	if manager.Count("wake:"+string(device)) != 2 {
		t.Fatal(manager.Calls())
	}
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	if manager.Running(device) {
		t.Fatal("idle Cloud runner still running")
	}
	result = w.turn("t2", "demi host shell --host Cloud 'cat report.txt'", "read the Cloud", 30000)
	if !strings.Contains(w.firstRequest, "[Execution target switched]") || !strings.Contains(result, "report") {
		t.Fatalf("request %s\nresult %s", w.firstRequest, result)
	}
	if manager.Count("wake:"+string(device)) != 3 {
		t.Fatal(manager.Calls())
	}
	s.cloudUntil(func(status webapi.CloudStatus) bool { return status.State == webapi.CloudStateOff })
	if manager.Running(device) {
		t.Fatal("idle Cloud runner still running")
	}
}

func TestResetHoldsCloudConversationButNotAttachedCloudTarget(t *testing.T) {
	s := newHostScenario(t, "demi-file")
	alpha := s.pair("alpha")
	work := s.work(cloudFirst)
	if result := work.turn("c1", "echo kept > note", "written"); !strings.Contains(result, "exitCode: 0") {
		t.Fatal(result)
	}
	device := s.theCloud()
	local := s.work(cloudSecond)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+cloudSecond, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, alpha.ID(), alpha.Runner.Home()), 200)
	read := conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", "/api/conversations/"+cloudSecond+"/hosts", "", 200)
	hosts, err := webapi.DecodeAttachedHosts(read.Body)
	if err != nil {
		t.Fatal(err)
	}
	if len(hosts.Hosts) != 1 || hosts.Hosts[0].DeviceID != device {
		t.Fatal(hosts)
	}
	held := s.manager.HoldReset(t)
	s.resetCloud(cloudReset)
	if err := held.UntilArrived(s.ctx, 1); err != nil {
		t.Fatal(err)
	}
	local.open()
	if result := local.turn("l1", "echo on-alpha", "done"); !strings.Contains(result, "on-alpha") {
		t.Fatal(result)
	}
	gate, err := backendtest.FileGate(s.ctx, s.b.Backend, s.user.User.ID, webapi.ConversationID(cloudFirst))
	if err != nil {
		t.Fatal(err)
	}
	opened := make(chan struct{})
	go func() {
		defer close(opened)
		work.open()
	}()
	defer func() {
		held.Release()
		<-opened
	}()
	for {
		count, changed := gate.TestingWaiting()
		if count > 0 {
			break
		}
		select {
		case <-opened:
			t.Fatal("open did not wait for reset")
		case <-changed:
		case <-s.ctx.Done():
			t.Fatal(s.ctx.Err())
		}
	}
	held.Release()
	<-opened
	if result := work.turn("c2", "cat note", "read"); !strings.Contains(result, "kept") {
		t.Fatal(result)
	}
}

func TestIdleConversationReleasesNativeResourcesOnRunningCloud(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	built, err := backendtest.BuildPackage(t.Context(), t, "demi-native-fixture")
	if err != nil {
		t.Fatal(err)
	}
	if err := h.UseNativeFixture(t.Context(), built); err != nil {
		t.Fatal(err)
	}
	h.Config.Lifecycle.IdleWindow = 600 * time.Millisecond
	h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	a, other := s.work(cloudFirst), s.work(cloudSecond)
	gate, err := backendtest.FileGate(s.ctx, b.Backend, user.User.ID, webapi.ConversationID(cloudSecond))
	if err != nil {
		t.Fatal(err)
	}
	working, err := gate.Enter(s.ctx, gates.Demand)
	if err != nil {
		t.Fatal(err)
	}
	defer working.Release()
	var results [2]string
	var workers sync.WaitGroup
	workers.Go(func() { results[0] = a.turn("a1", "echo ran", "ran") })
	workers.Go(func() { results[1] = other.turn("b1", "echo ran", "ran") })
	workers.Wait()
	for _, result := range results {
		if !strings.Contains(result, "exitCode: 0") {
			t.Fatal(result)
		}
	}
	for _, id := range []string{cloudFirst, cloudSecond} {
		_, code, reason := conversationStreamEnd(s.ctx, t, conversationStream(s.ctx, t, s.b, &s.user, id, "retain"))
		if code != int(websocket.StatusNormalClosure) || reason != "completed" {
			t.Fatalf("stream close: %d %s", code, reason)
		}
	}
	firstGate, err := backendtest.FileGate(s.ctx, b.Backend, user.User.ID, webapi.ConversationID(cloudFirst))
	if err != nil {
		t.Fatal(err)
	}
	for {
		changed := firstGate.State().Changed()
		held, _, _ := conversationStreamEnd(s.ctx, t, conversationStream(s.ctx, t, s.b, &s.user, cloudSecond, "held"))
		if string(held) == fmt.Sprintf(`{"conversations":[%q]}`, cloudSecond) {
			break
		}
		select {
		case <-changed:
		case <-s.ctx.Done():
			t.Fatal(s.ctx.Err())
		}
	}
	device := s.theCloud()
	if manager.Count("wake:"+string(device)) != 1 || manager.Count("hibernate:"+string(device)) != 0 {
		t.Fatal(manager.Calls())
	}
	if s.cloudStatus().State != webapi.CloudStateRunning {
		t.Fatal("Cloud stopped")
	}
	working.Release()
}

// Two idle windows of actual file requests prove activity on a paired target
// holds its attached Cloud; requests are event waits, without polling sleeps.
func TestPairedTargetActivityKeepsAttachedCloudAwake(t *testing.T) {
	const window = 600 * time.Millisecond
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Lifecycle.IdleWindow = window
	h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	paired := s.pair("paired")
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	listing := "/api/conversations/" + cloudFirst + "/fs"
	conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 200)
	device := s.theCloud()
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+cloudFirst, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, paired.ID(), paired.Runner.Home()), 200)
	working := time.Now()
	rested := working
	for rested.Sub(working) < 2*window {
		rested = time.Now()
		conversationRequest(s.ctx, s.t, s.b, &s.user, "GET", listing, "", 200)
	}
	stopped, err := manager.Arrival(s.ctx, "hibernate:"+string(device))
	if err != nil {
		t.Fatal(err)
	}
	if stopped.Before(rested.Add(window)) {
		t.Fatalf("Cloud stopped %s after work began; last request at %s", stopped.Sub(working), rested.Sub(working))
	}
}

func TestResetHoldsPairedConversationWhoseProviderUsesCloud(t *testing.T) {
	h, manager, err := backendtest.HostsHarness(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Families.Register("process", backendtest.HostsProcessFamily{Test: t})
	b, user, err := h.StartSetUp(t.Context(), t)
	if err != nil {
		t.Fatal(err)
	}
	s := &hostScenario{t, t.Context(), h, b, user, manager}
	alpha := s.pair("alpha")
	answer := conversationRequest(s.ctx, s.t, s.b, &s.user, "POST", "/api/providers", `{"source":"custom","providerType":"process","label":"Process","apiKey":"k","models":[{"id":"m","displayName":"M","contextWindow":100000,"outputLimit":null,"thinkingEfforts":[],"acceptedExtensions":null,"fastTier":null}]}`, 201)
	entry, err := webapi.DecodeProviderAnswer(answer.Body)
	if err != nil {
		t.Fatal(err)
	}
	conversationCreate(s.ctx, s.t, s.b, &s.user, cloudFirst)
	conversationRequest(s.ctx, s.t, s.b, &s.user, "PATCH", "/api/conversations/"+cloudFirst, fmt.Sprintf(`{"target":{"kind":"device","deviceId":%q,"path":%q}}`, alpha.ID(), alpha.Runner.Home()), 200)
	conversationChoose(s.ctx, t, s.b, &s.user, cloudFirst, string(entry.Provider.ID), "m")
	open := func() {
		socket := conversationOpen(s.ctx, t, b, &user, cloudFirst)
		wireMust(t, socket.Close(s.ctx))
	}

	open()
	hold := manager.HoldReset(t)
	s.resetCloud(cloudReset)
	if err := hold.UntilArrived(s.ctx, 1); err != nil {
		t.Fatal(err)
	}
	gate, err := backendtest.FileGate(s.ctx, b.Backend, user.User.ID, webapi.ConversationID(cloudFirst))
	if err != nil {
		t.Fatal(err)
	}
	opened := make(chan struct{})
	go func() {
		defer close(opened)
		open()
	}()
	defer func() {
		hold.Release()
		<-opened
	}()
	for {
		count, changed := gate.TestingWaiting()
		if count > 0 {
			break
		}
		select {
		case <-opened:
			t.Fatal("open did not wait for reset")
		case <-changed:
		case <-s.ctx.Done():
			t.Fatal(s.ctx.Err())
		}
	}
	hold.Release()
	<-opened
}
