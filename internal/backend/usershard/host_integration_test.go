package usershard_test

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
	"mvdan.cc/sh/v3/shell"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

func inertRunner(t *testing.T, f *fixture, device webapiproto.DeviceID) {
	t.Helper()
	link, driver := remotehost.NewLink(
		remotehost.LinkOptions{
			Device: string(device),
			Identity: host.Identity{
				UID:      501,
				GID:      20,
				Hostname: "test",
				HomeDir:  "/home/ana",
			},
			Pipes: f.shard.Pipes(),
		},
	)
	serving := f.shard.Devices().Bind(device, link, driver, runners.NewLastSeen(f.services.Control, f.shard.Marks()))
	t.Cleanup(func() {
		serving.Close(context.Background())
	})
}

func TestHostGroupNamesReachableHostsAndRefusesOthers(t *testing.T) {
	f := shardFixture(t, "laptop", "ci")
	laptop, ci := f.devices[0].ID, f.devices[1].ID
	change(t, f, on(laptop))
	change(
		t,
		f,
		&database.ConversationRecordChange{
			Change: &database.RecordAttach{Host: database.AttachedHostRecord{Device: ci, Name: "ci"}},
		},
	)
	inertRunner(t, f, laptop)
	commands := &host.CommandSet{}
	if err := commands.Register(host.Group("demi", "Demi.", hostaccess.HostGroup(f.shard))); err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		leaf, args  string
		code        uint8
		output, err string
	}{
		{
			"list",
			`{}`,
			0,
			fmt.Sprintf("laptop  %s  online  /work  (main)\nci  %s  offline  ?  (attached)\n", laptop, ci),
			"",
		},
		{
			"current",
			`{}`,
			0,
			fmt.Sprintf("host: machine \"laptop\" (%s, online) — /work\n", laptop),
			"",
		},
		{
			"shell",
			`{"host":"elsewhere","script":"pwd"}`,
			1,
			"",
			"host shell: host elsewhere is not reachable from this conversation (see `demi host list`)\n",
		},
		{
			"shell",
			`{"host":"ci","script":"  "}`,
			2,
			"",
			"usage: demi host shell --host <name|id> <script>\n",
		},
		{
			"shell",
			`{"host":"ci","script":"pwd"}`,
			1,
			"",
			"host shell: cross-host execution requires a machine job\n",
		},
	}
	for _, test := range cases {
		port := hosttest.NewMemoryPort(nil)
		invocation := host.RPCInvocation{
			Path: []string{"demi", "host", test.leaf},
			Args: []byte(test.args),
			CWD:  "/work",
			Env:  map[string]string{},
			Context: commandproto.Context{
				Conversation: string(conversationID),
				Caller:       &commandproto.AgentCaller{Number: 1},
				Locale:       runners.DefaultLocale(),
			},
			Caller: &host.JobCaller{Node: "node-1"},
		}
		code, err := commands.Dispatch(t.Context(), invocation, port.Port())
		if err != nil {
			t.Fatal(err)
		}
		if code != test.code || string(port.Stdout()) != test.output || string(port.Stderr()) != test.err {
			t.Errorf(
				"%s %s = (%d,%q,%q), want (%d,%q,%q)",
				test.leaf,
				test.args,
				code,
				port.Stdout(),
				port.Stderr(),
				test.code,
				test.output,
				test.err,
			)
		}
	}
}

func TestJobRunsWithinHostAccessAndRejectsChangedHost(t *testing.T) {
	f := shardFixture(t, "laptop")
	laptop := f.devices[0].ID
	change(t, f, on(laptop))
	inertRunner(t, f, laptop)
	remote, err := hostaccess.ConversationHostForNode(t.Context(), f.shard, conversationID)
	if err != nil {
		t.Fatal(err)
	}
	if remote.DefaultCWD() != "/work" {
		t.Fatalf("cwd = %s", remote.DefaultCWD())
	}
	ran := 0
	job := func(context.Context) error {
		if f.shard.Conversations().Slot(conversationID).FileGate().State().Demand == 0 {
			t.Error("job ran outside file admission")
		}
		ran++
		return nil
	}
	if err := hostaccess.RunJob(t.Context(), f.shard, conversationID, remote.Key(), job); err != nil {
		t.Fatal(err)
	}
	record, _, err := f.services.Control.Conversation(t.Context(), conversationID)
	if err != nil {
		t.Fatal(err)
	}
	changed, err := f.services.Control.SwitchConversationTarget(
		t.Context(),
		conversationID,
		record.Target,
		&webapiproto.ConversationTargetDevice{DeviceID: laptop, Path: "/elsewhere"},
		database.TargetSwitch{
			From: &database.ExecutionDevice{DeviceID: laptop, Path: "/work"},
			To:   &database.ExecutionDevice{DeviceID: laptop, Path: "/elsewhere"},
		},
		database.SwitchEnds{},
	)
	if err != nil || !changed {
		t.Fatalf("change binding = %v, %v", changed, err)
	}
	err = hostaccess.RunJob(t.Context(), f.shard, conversationID, remote.Key(), job)
	var refused *host.Error
	if !errors.As(err, &refused) || refused.Kind != host.Unavailable {
		t.Fatalf("old Host job = %v", err)
	}
	if ran != 1 {
		t.Fatalf("job ran %d times", ran)
	}
}

func TestRemoteReferencePreservesDeviceAndPathAndRefusesWholeBatch(t *testing.T) {
	f := shardFixture(t, "build")
	account, _, err := f.services.Control.Account(t.Context(), f.owner)
	if err != nil {
		t.Fatal(err)
	}
	other, err := f.services.Control.CreateUser(
		t.Context(),
		webapiproto.EmailAddress("other@example.test"),
		account.PasswordHash,
		webapiproto.RoleUser,
	)
	if err != nil {
		t.Fatal(err)
	}
	foreign, err := f.services.Control.CreateDevice(
		t.Context(),
		other.ID,
		"foreign",
		runnerproto.RunnerPlatformLinux,
		database.HashToken("foreign"),
	)
	if err != nil {
		t.Fatal(err)
	}
	build := f.devices[0].ID
	path := "/srv/it's $(literal).txt"
	file := func(id webapiproto.DeviceID) hostaccess.RemoteFile {
		return hostaccess.RemoteFile{Device: string(id), Path: path}
	}
	_, err = hostaccess.ReferenceRemoteFiles(t.Context(), f.shard, conversationID, []hostaccess.RemoteFile{file(build)})
	if err == nil || err.Error() != "Referenced device "+f.devices[0].Name+" is offline" {
		t.Fatalf("offline reference = %v", err)
	}
	inertRunner(t, f, build)
	_, err = hostaccess.ReferenceRemoteFiles(
		t.Context(),
		f.shard,
		conversationID,
		[]hostaccess.RemoteFile{file(build), file(foreign.ID)},
	)
	if !errors.Is(err, hostaccess.ErrDeviceNotAccessible) {
		t.Fatalf("foreign batch = %v", err)
	}
	before, err := f.services.Control.AttachedHosts(t.Context(), conversationID)
	if err != nil {
		t.Fatal(err)
	}
	if len(before) != 0 {
		t.Fatal("refused batch attached a device")
	}
	blocks, err := hostaccess.ReferenceRemoteFiles(
		t.Context(),
		f.shard,
		conversationID,
		[]hostaccess.RemoteFile{file(build)},
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(blocks) != 1 {
		t.Fatalf("references = %v", blocks)
	}
	reference, ok := blocks[0].(*types.UserReference)
	if !ok {
		t.Fatalf("reference = %T", blocks[0])
	}
	parsed, err := url.Parse(reference.Reference)
	if err != nil {
		t.Fatal(err)
	}
	if parsed.Path != path || parsed.Query().Get("deviceId") != string(build) || parsed.Query().Get("host") != "build" {
		t.Fatalf("reference lost identity: %s", reference.Reference)
	}
	// Parse both shell levels, so quoting may vary but cannot evaluate the path.
	command, err := shell.Fields(parsed.Query().Get("readCommand"), func(string) string {
		return ""
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(command) != 6 {
		t.Fatalf("read command words = %q", command)
	}
	if diff := cmp.Diff([]string{
		"demi",
		"host",
		"shell",
		"--host",
		string(build),
	}, command[:5]); diff != "" {
		t.Fatal(diff)
	}
	read, err := shell.Fields(command[5], func(string) string {
		return ""
	})
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff([]string{"cat", "--", path}, read); diff != "" {
		t.Fatal(diff)
	}

	attached, err := f.services.Control.AttachedHosts(t.Context(), conversationID)
	if err != nil {
		t.Fatal(err)
	}
	if len(attached) != 1 || attached[0].Device != build {
		t.Fatalf("attachments = %+v", attached)
	}
	record, _, err := f.services.Control.Conversation(t.Context(), conversationID)
	if err != nil {
		t.Fatal(err)
	}
	if record.ContextVersion != 1 {
		t.Fatalf("context revision = %d", record.ContextVersion)
	}
}

func TestCloudGrowthRequiresPositiveBoundedCloudVolume(t *testing.T) {
	f := shardFixture(t, "laptop")
	managed, err := f.services.Control.ManagedDeviceOrCreate(t.Context(), f.owner)
	if err != nil {
		t.Fatal(err)
	}
	tuning := f.services.Cloud.Tuning
	cases := []struct {
		device  webapiproto.DeviceID
		volume  runnerproto.VolumeName
		bytes   uint64
		message string
	}{
		{
			managed.ID,
			runnerproto.VolumeNameSystem,
			0,
			"system volume quota exceeded",
		},
		{
			managed.ID,
			runnerproto.VolumeNameSystem,
			tuning.SystemQuota + 1,
			"system volume quota exceeded",
		},
		{
			managed.ID,
			runnerproto.VolumeNameHome,
			tuning.HomeQuota + 1,
			"home volume quota exceeded",
		},
		{
			f.devices[0].ID,
			runnerproto.VolumeNameHome,
			1 << 30,
			"Only the Cloud grows its volumes",
		},
	}
	for _, test := range cases {
		err := cloud.GrowVolume(t.Context(), f.shard, test.device, test.volume, test.bytes)
		if err == nil || err.Error() != test.message {
			t.Errorf("grow(%s,%s,%d) = %v", test.device, test.volume, test.bytes, err)
		}
	}
	err = cloud.GrowVolume(t.Context(), f.shard, managed.ID, runnerproto.VolumeNameHome, tuning.HomeQuota)
	if err == nil || !strings.HasPrefix(err.Error(), "Machine manager unavailable during grow_volume: ") {
		t.Fatalf("bounded growth did not reach manager: %v", err)
	}
}
