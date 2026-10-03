package plugins_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// portFixture obtains user and conversation ports through ordinary page calls.
func portFixture(t *testing.T) (*plugins.User, *fakeShard, plugin.Port, plugin.Port) {
	t.Helper()
	m := manifest(t, "storage")
	m.Page.Methods[1].Operations = []declare.NativeOperation{{Package: "allowed", Operation: "declared"}}
	var ports []plugin.Port
	f := &fakeFactory{manifest: m, make: func() plugin.Plugin {
		return &fakePlugin{call: func(_ context.Context, _ plugin.Request, port plugin.Port) (plugin.Reply, error) {
			ports = append(ports, port)
			return &plugin.ReplyResult{Result: json.RawMessage(`{}`)}, nil
		}}
	}}
	u, shard := userFixture(t, registry(t, f, &fakeFactory{manifest: manifest(t, "other")}))
	if _, err := u.PageCall(
		t.Context(),
		plugins.PageCall{Plugin: "storage", Method: "user", Params: []byte(`{}`)},
	); err != nil {
		t.Fatal(err)
	}
	if _, err := u.PageCall(
		t.Context(),
		plugins.PageCall{
			Plugin:       "storage",
			Method:       "conversation",
			Params:       []byte(`{}`),
			Conversation: new(webapi.ConversationID("conversation")),
		},
	); err != nil {
		t.Fatal(err)
	}
	return u, shard, ports[0], ports[1]
}

func TestStoragePortsKeepValuesBlobsAndDirectories(t *testing.T) {
	u, shard, port, _ := portFixture(t)
	ctx := t.Context()
	value := json.RawMessage("{\"z\":\"<&\u2028\u2029\",\"a\":1}")
	blob := core.BlobRef(strings.Repeat("a", 64))
	if _, found, err := port.Value(ctx, "key"); err != nil || found {
		t.Fatalf("%v %v", found, err)
	}
	revision, err := port.WriteValueNaming(ctx, "key", value, nil, []core.BlobRef{blob})
	if err != nil || revision != 1 {
		t.Fatalf("%d %v", revision, err)
	}
	stored, found, err := port.Value(ctx, "key")
	if err != nil || !found || stored.Revision != 1 || string(stored.Value) != string(value) {
		t.Fatalf("%+v %v %v", stored, found, err)
	}
	values, err := port.Values(ctx)
	if err != nil || len(values) != 1 || string(values["key"].Value) != string(value) {
		t.Fatalf("%+v %v", values, err)
	}
	// The same key is isolated from another plugin and another user.
	other, err := shard.control.PluginValue(ctx, shard.user, "other", "key")
	if err != nil || other != nil {
		t.Fatalf("cross-plugin read: %v %v", other, err)
	}
	other, err = shard.control.PluginValue(ctx, "another-user", "storage", "key")
	if err != nil || other != nil {
		t.Fatalf("cross-user read: %v %v", other, err)
	}
	if _, err := port.WriteValue(ctx, "key", []byte(`null`), nil); err == nil {
		t.Fatal("missing revision overwrote value")
	}
	start := make(chan struct{})
	outcomes := make(chan error, 2)
	for range 2 {
		go func() {
			<-start
			_, err := port.WriteValue(ctx, "key", value, &revision)
			outcomes <- err
		}()
	}
	close(start)
	successes, conflicts := 0, 0
	for range 2 {
		err := <-outcomes
		var conflict *plugin.PortRefusalConflict
		if err == nil {
			successes++
		} else if errors.As(err, &conflict) {
			conflicts++
		} else {
			t.Fatal(err)
		}
	}
	if successes != 1 || conflicts != 1 {
		t.Fatalf("writes: %d successes, %d conflicts", successes, conflicts)
	}
	var conflict *plugin.PortRefusalConflict
	if err := port.RemoveValue(ctx, "key", 1); !errors.As(err, &conflict) {
		t.Fatalf("%v", err)
	}
	if err := port.RemoveValue(ctx, "key", 2); err != nil {
		t.Fatal(err)
	}
	if values, err := port.Values(ctx); err != nil || len(values) != 0 {
		t.Fatalf("%v %v", values, err)
	}
	directories := []plugin.HostDirectory{
		{
			Name:  "docs",
			Files: []plugin.DirectoryFile{{Path: "SKILL.md", Blob: blob}},
		},
	}
	paths, err := port.SetDirectories(ctx, directories)
	if err != nil || len(paths) != 1 || paths[0].Path != directories[0].Path("storage") {
		t.Fatalf("%v %v", paths, err)
	}
	installed, err := u.Directories(ctx)
	if err != nil || !reflect.DeepEqual(installed[0].Directories, directories) || len(installed[1].Directories) != 0 {
		t.Fatalf("%+v %v", installed, err)
	}
	if _, err := port.SetDirectories(ctx, []plugin.HostDirectory{directories[0], directories[0]}); err == nil {
		t.Fatal("duplicate directories accepted")
	}
	if _, err := port.SetDirectories(
		ctx,
		[]plugin.HostDirectory{
			{
				Name:  "bad",
				Files: []plugin.DirectoryFile{{Path: "../escape", Blob: blob}},
			},
		},
	); err == nil {
		t.Fatal("escaping file accepted")
	}
	failure := errors.New("blob is being collected")
	shard.uses.fail = failure
	if _, err := port.WriteValueNaming(ctx, "refused", value, nil, []core.BlobRef{blob}); !errors.Is(err, failure) {
		t.Fatalf("blob refusal lost: %v", err)
	}
	if _, err := port.SetDirectories(ctx, nil); !errors.Is(err, failure) {
		t.Fatalf("directory refusal lost: %v", err)
	}
	installed, err = u.Directories(ctx)
	if err != nil || !reflect.DeepEqual(installed[0].Directories, directories) {
		t.Fatalf("failed write changed directories: %+v %v", installed, err)
	}
	shard.uses.fail = nil
	if len(shard.uses.touched) == 0 || shard.uses.touched[0] != blob {
		t.Fatal("value never recorded blob uses")
	}
	if _, err := u.Switch(ctx, "storage", false); err != nil {
		t.Fatal(err)
	}
	installed, err = u.Directories(ctx)
	if err != nil || len(installed[0].Directories) != 0 {
		t.Fatalf("disabled directories: %+v %v", installed, err)
	}
	if _, err := u.Switch(ctx, "storage", true); err != nil {
		t.Fatal(err)
	}
	installed, err = u.Directories(ctx)
	if err != nil || !reflect.DeepEqual(installed[0].Directories, directories) {
		t.Fatalf("restored directories: %+v %v", installed, err)
	}
}

func TestProductPortsForwardServicesAndRefusals(t *testing.T) {
	_, shard, userPort, conversationPort := portFixture(t)
	ctx := t.Context()
	ref := core.BlobRef(strings.Repeat("b", 64))
	bytes := core.B64Bytes("blob bytes")
	record := plugin.ExposeRecord{
		ID:     "expose",
		Device: "device",
		URL:    "https://example.test",
	}
	hostList := []plugin.ConversationHost{{
		Name:   "main",
		Device: "device",
		Role:   plugin.HostRoleMain,
		Online: true,
	}}
	reads := []plugin.HostRead{{Path: "notes"}}
	files := []plugin.HostFile{}
	var calls []string
	shard.put = func(_ context.Context, got core.B64Bytes) (core.BlobRef, error) {
		if !reflect.DeepEqual(got, bytes) {
			t.Error(got)
		}
		calls = append(calls, "put")
		return ref, nil
	}
	shard.blob = func(_ context.Context, got core.BlobRef) (*core.B64Bytes, error) {
		if got != ref {
			t.Error(got)
		}
		calls = append(calls, "blob")
		return &bytes, nil
	}
	shard.hosts = func(_ context.Context, c webapi.ConversationID) ([]plugin.ConversationHost, error) {
		if c != "conversation" {
			t.Error(c)
		}
		calls = append(calls, "hosts")
		return hostList, nil
	}
	shard.files = func(
		_ context.Context,
		c webapi.ConversationID,
		got []plugin.HostRead,
	) ([]plugin.HostFile, error) {
		if c != "conversation" || !reflect.DeepEqual(got, reads) {
			t.Errorf("%s %+v", c, got)
		}
		calls = append(calls, "files")
		return files, nil
	}
	shard.exposes = func(context.Context) (plugin.ExposeList, error) {
		calls = append(calls, "exposes")
		return plugin.ExposeList{Available: true, Exposes: []plugin.ExposeRecord{record}}, nil
	}
	shard.create = func(
		_ context.Context,
		d webapi.DeviceID,
		address string,
		lifetime uint64,
	) (plugin.ExposeRecord, error) {
		if d != "device" || address != "127.0.0.1:80" || lifetime != 60 {
			t.Errorf("%s %s %d", d, address, lifetime)
		}
		calls = append(calls, "create")
		return record, nil
	}
	shard.renew = func(_ context.Context, id webapi.ExposeID, lifetime uint64) (plugin.ExposeRecord, error) {
		if id != "expose" || lifetime != 90 {
			t.Errorf("%s %d", id, lifetime)
		}
		calls = append(calls, "renew")
		return record, nil
	}
	shard.remove = func(_ context.Context, id webapi.ExposeID) error {
		if id != "expose" {
			t.Error(id)
		}
		calls = append(calls, "remove")
		return nil
	}
	raw := json.RawMessage(`{"z":1,"a":"<&"}`)
	// Calls are allowed for the package, not just the one declared operation.
	operation := declare.NativeOperation{Package: "allowed", Operation: "another"}
	shard.packageCall = func(
		_ context.Context,
		c webapi.ConversationID,
		receivedOperation declare.NativeOperation,
		args json.RawMessage,
		kind plugin.CallKind,
	) (json.RawMessage, error) {
		if c != "conversation" || receivedOperation != operation || string(args) != string(raw) ||
			kind != plugin.CallKindLooks {
			t.Errorf("%s %+v %s %s", c, receivedOperation, args, kind)
		}
		calls = append(calls, "package")
		return raw, nil
	}
	tests := []struct {
		port    plugin.Port
		message plugin.PortMessage
		answer  plugin.PortAnswer
	}{
		{
			userPort,
			&plugin.PortMessagePutBlob{Bytes: bytes},
			&plugin.PortAnswerBlob{Blob: ref},
		},
		{
			userPort,
			&plugin.PortMessageGetBlob{Blob: ref},
			&plugin.PortAnswerBytes{Bytes: &bytes},
		},
		{
			conversationPort,
			&plugin.PortMessageConversationHosts{},
			&plugin.PortAnswerHosts{Hosts: hostList},
		},
		{
			conversationPort,
			&plugin.PortMessageReadHostFiles{Reads: reads},
			&plugin.PortAnswerHostFiles{Files: files},
		},
		{
			userPort,
			&plugin.PortMessageListExposes{},
			&plugin.PortAnswerExposes{
				List: plugin.ExposeList{Available: true, Exposes: []plugin.ExposeRecord{record}},
			},
		},
		{
			userPort,
			&plugin.PortMessageCreateExpose{Device: "device", Address: "127.0.0.1:80", Lifetime: 60},
			&plugin.PortAnswerExpose{Expose: record},
		},
		{
			userPort,
			&plugin.PortMessageRenewExpose{Expose: "expose", Lifetime: 90},
			&plugin.PortAnswerExpose{Expose: record},
		},
		{
			userPort,
			&plugin.PortMessageRemoveExpose{Expose: "expose"},
			&plugin.PortAnswerDone{},
		},
		{
			conversationPort,
			&plugin.PortMessagePackageCall{
				Operation: operation,
				Args:      raw,
				Kind:      plugin.CallKindLooks,
			},
			&plugin.PortAnswerCalled{Result: raw},
		},
	}
	for _, scenario := range tests {
		got, err := scenario.port.Forward(ctx, scenario.message)
		if err != nil || !reflect.DeepEqual(got, scenario.answer) {
			t.Fatalf("%T: %+v %v", scenario.message, got, err)
		}
	}
	if !reflect.DeepEqual(
		calls,
		[]string{
			"put",
			"blob",
			"hosts",
			"files",
			"exposes",
			"create",
			"renew",
			"remove",
			"package",
		},
	) {
		t.Fatal(calls)
	}
	for _, message := range []plugin.PortMessage{
		&plugin.PortMessageConversationHosts{},
		&plugin.PortMessageReadHostFiles{Reads: reads},
		&plugin.PortMessagePackageCall{
			Operation: operation,
			Args:      raw,
			Kind:      plugin.CallKindLooks,
		},
		&plugin.PortMessageChanged{Scope: plugin.ScopeConversation},
	} {
		answer, err := userPort.Forward(ctx, message)
		refused, ok := answer.(*plugin.PortAnswerRefused)
		if err != nil || !ok {
			t.Fatalf("%T: %+v %v", message, answer, err)
		}
		if _, ok := refused.Refusal.(*plugin.PortRefusalNoConversation); !ok {
			t.Fatalf("%T", refused.Refusal)
		}
	}
	for _, message := range []plugin.PortMessage{
		&plugin.PortMessageRPC{Request: &host.PortStdout{Bytes: bytes}},
		&plugin.PortMessagePackageCall{
			Operation: declare.NativeOperation{Package: "foreign", Operation: "run"},
			Args:      raw,
			Kind:      plugin.CallKindStarts,
		},
	} {
		_, err := userPort.Forward(ctx, message)
		var unexpected *host.PortError
		if !errors.As(err, &unexpected) || unexpected.Kind != host.UnexpectedReply {
			t.Fatal(err)
		}
	}
	refusal := &plugin.PortRefusalNotRunning{}
	shard.files = func(context.Context, webapi.ConversationID, []plugin.HostRead) ([]plugin.HostFile, error) {
		return nil, fmt.Errorf("host access: %w", refusal)
	}
	if _, err := conversationPort.ReadHostFiles(ctx, reads); !errors.Is(err, refusal) {
		t.Fatalf("host refusal lost: %v", err)
	}
	failure := &host.PortError{Kind: host.PortFailed, Message: "relay failed"}
	shard.remove = func(context.Context, webapi.ExposeID) error { return failure }
	if err := userPort.RemoveExpose(ctx, "expose"); !errors.Is(err, failure) {
		t.Fatalf("product error lost: %v", err)
	}
}
