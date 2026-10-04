package plugintest

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

// PackageCalls answers a test's package calls. A refusal can be returned as an error.
type PackageCalls func(
	context.Context,
	declare.NativeOperation,
	json.RawMessage,
	plugin.CallKind,
) (json.RawMessage, error)

// PackageCall records a package call made by the plugin.
type PackageCall struct {
	// Operation identifies the called package operation.
	Operation declare.NativeOperation
	// Args contains the call's JSON input.
	Args json.RawMessage
	// Kind records the Host access the call needs.
	Kind plugin.CallKind
}

// TestDemi answers a plugin port from memory. Configure exported fields only
// while no requests are running; its inspection methods are concurrency-safe.
type TestDemi struct {
	// Plugin names the plugin whose port is being tested.
	Plugin plugin.ID
	// RPC answers command transport requests.
	RPC host.PortTransport
	// HostFiles supplies the running Host's file contents.
	HostFiles map[string][]byte
	// Hosts lists the conversation's available Hosts.
	Hosts []plugin.ConversationHost
	// ExposesAvailable reports whether the test expose domain is available.
	ExposesAvailable bool
	// Now supplies the test clock for expose lifetimes.
	Now core.Timestamp
	// PackageCalls answers native package operations.
	PackageCalls PackageCalls
	// Panel answers scripted panel operations.
	Panel plugin.Transport

	mu          sync.Mutex
	values      map[string]plugin.StoredValue
	valueBlobs  map[string][]core.BlobRef
	blobs       map[core.BlobRef]core.B64Bytes
	directories []plugin.HostDirectory
	exposes     []plugin.ExposeRecord
	exposesMade uint64
	called      []PackageCall
	changed     uint32
	answered    chan struct{}
}

// New constructs Demi with no running Host and an available expose domain.
func New() *TestDemi { return WithRPC(nil) }

// WithRPC constructs Demi with the given command transport.
func WithRPC(rpc host.PortTransport) *TestDemi {
	return &TestDemi{
		Plugin: "test", RPC: rpc, ExposesAvailable: true, Now: core.UnixEpoch,
		values: make(map[string]plugin.StoredValue), valueBlobs: make(map[string][]core.BlobRef),
		blobs: make(map[core.BlobRef]core.B64Bytes), answered: make(chan struct{}),
	}
}

// Port returns a port over this Demi; the request context owns cancellation.
func (d *TestDemi) Port() plugin.Port { return plugin.NewPort(d) }

// Until checks after each non-rpc message until check holds or ctx ends.
func (d *TestDemi) Until(ctx context.Context, check func(*TestDemi) bool) error {
	for {
		d.mu.Lock()
		answered := d.answered
		d.mu.Unlock()
		if check(d) {
			return nil
		}
		select {
		case <-answered:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// Value returns an independent copy of the value stored under key; ok is false when there is none.
func (d *TestDemi) Value(key string) (plugin.StoredValue, bool) {
	d.mu.Lock()
	defer d.mu.Unlock()
	v, ok := d.values[key]
	if !ok {
		return plugin.StoredValue{}, false
	}
	v.Value = bytes.Clone(v.Value)
	return v, true
}

// ValueBlobs returns the blobs retained by a value.
func (d *TestDemi) ValueBlobs(key string) []core.BlobRef {
	d.mu.Lock()
	defer d.mu.Unlock()
	return slices.Clone(d.valueBlobs[key])
}

// BlobBytes returns a copy of the blob's bytes; ok is false when the blob is unknown.
func (d *TestDemi) BlobBytes(blob core.BlobRef) (core.B64Bytes, bool) {
	d.mu.Lock()
	defer d.mu.Unlock()
	value, ok := d.blobs[blob]
	if !ok {
		return nil, false
	}
	return slices.Clone(value), true
}

// Directories returns the plugin's current directory set.
func (d *TestDemi) Directories() []plugin.HostDirectory {
	d.mu.Lock()
	defer d.mu.Unlock()
	return cloneDirectories(d.directories)
}

// Changes returns the number of page-state change messages.
func (d *TestDemi) Changes() uint32 {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.changed
}

// Called returns the package calls in order.
func (d *TestDemi) Called() []PackageCall {
	d.mu.Lock()
	defer d.mu.Unlock()
	calls := slices.Clone(d.called)
	for i := range calls {
		calls[i].Args = bytes.Clone(calls[i].Args)
	}
	return calls
}

// Request answers one port message. No mutex is held while a transport or
// package callback runs.
func (d *TestDemi) Request(ctx context.Context, message plugin.PortMessage) (plugin.PortAnswer, error) {
	switch any(message).(type) {
	case *plugin.PortMessagePanelTabs,
		*plugin.PortMessageCreatePanelTab,
		*plugin.PortMessageUpdatePanelTab,
		*plugin.PortMessageRemovePanelTab:
		if d.Panel == nil {
			return nil, fmt.Errorf("no scripted panel")
		}
		return d.Panel.Request(ctx, message)
	}
	if m, ok := message.(*plugin.PortMessageRPC); ok {
		if d.RPC == nil {
			return nil, fmt.Errorf("the test gives an rpc transport")
		}
		response, err := d.RPC.Request(ctx, m.Request)
		if err != nil {
			return nil, err
		}
		return &plugin.PortAnswerRPC{Response: response}, nil
	}
	if m, ok := message.(*plugin.PortMessagePackageCall); ok {
		if d.PackageCalls == nil {
			return nil, fmt.Errorf("the test answers package calls")
		}
		result, err := d.PackageCalls(ctx, m.Operation, bytes.Clone(m.Args), m.Kind)
		d.mu.Lock()
		d.called = append(d.called, PackageCall{Operation: m.Operation, Args: bytes.Clone(m.Args), Kind: m.Kind})
		d.notifyLocked()
		d.mu.Unlock()
		if err != nil {
			var refusal plugin.PortRefusal
			if errors.As(err, &refusal) {
				return &plugin.PortAnswerRefused{Refusal: refusal}, nil
			}
			return nil, err
		}
		return &plugin.PortAnswerCalled{Result: bytes.Clone(result)}, nil
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	defer d.notifyLocked()
	return d.answerLocked(message)
}

// notifyLocked wakes every inspection waiter after a port message while mu is held.
func (d *TestDemi) notifyLocked() {
	close(d.answered)
	d.answered = make(chan struct{})
}

// answerLocked performs the in-memory portion of a plugin operation while mu is held.
func (d *TestDemi) answerLocked(message plugin.PortMessage) (plugin.PortAnswer, error) {
	switch m := message.(type) {
	case *plugin.PortMessageReadValue:
		return d.readValueLocked(m)
	case *plugin.PortMessageListValues:
		return d.listValuesLocked(m)
	case *plugin.PortMessageWriteValue:
		return d.writeValueLocked(m)
	case *plugin.PortMessageRemoveValue:
		current, exists := d.values[m.Key]
		if !exists || current.Revision != m.Revision {
			return refused(&plugin.PortRefusalConflict{}), nil
		}
		delete(d.values, m.Key)
		delete(d.valueBlobs, m.Key)
		return &plugin.PortAnswerDone{}, nil
	case *plugin.PortMessagePutBlob:
		blob := core.BlobRefOf(m.Bytes)
		d.blobs[blob] = slices.Clone(m.Bytes)
		return &plugin.PortAnswerBlob{Blob: blob}, nil
	case *plugin.PortMessageGetBlob:
		return d.getBlobLocked(m)
	case *plugin.PortMessageSetDirectories:
		return d.setDirectoriesLocked(m)
	case *plugin.PortMessageReadHostFiles:
		return d.readHostFilesLocked(m)
	case *plugin.PortMessageChanged:
		d.changed++
		return &plugin.PortAnswerDone{}, nil
	case *plugin.PortMessageConversationHosts:
		return &plugin.PortAnswerHosts{Hosts: append([]plugin.ConversationHost{}, d.Hosts...)}, nil
	case *plugin.PortMessageListExposes:
		return d.listExposesLocked(m)
	case *plugin.PortMessageCreateExpose:
		return d.createExposeLocked(m)
	case *plugin.PortMessageRenewExpose:
		return d.renewExposeLocked(m)
	case *plugin.PortMessageRemoveExpose:
		return d.removeExposeLocked(m)
	case *plugin.PortMessagePanelTabs,
		*plugin.PortMessageCreatePanelTab,
		*plugin.PortMessageUpdatePanelTab,
		*plugin.PortMessageRemovePanelTab,
		*plugin.PortMessageRPC, *plugin.PortMessagePackageCall:
		return nil, fmt.Errorf("callback operation must run outside the memory lock")
	}
	return nil, fmt.Errorf("unknown plugin port message")
}

// refused puts a domain refusal on the plugin wire.
func refused(r plugin.PortRefusal) plugin.PortAnswer { return &plugin.PortAnswerRefused{Refusal: r} }

// cloneDirectories detaches the memory transport's directory declarations.
func cloneDirectories(directories []plugin.HostDirectory) []plugin.HostDirectory {
	result := slices.Clone(directories)
	for i := range result {
		result[i].Files = slices.Clone(result[i].Files)
	}
	return result
}

// writeValueLocked answers the memory port operation while mu is held.
func (d *TestDemi) writeValueLocked(m *plugin.PortMessageWriteValue) (plugin.PortAnswer, error) {
	current, exists := d.values[m.Key]
	if exists != (m.Revision != nil) || exists && current.Revision != *m.Revision {
		return refused(&plugin.PortRefusalConflict{}), nil
	}
	for _, blob := range m.Blobs {
		if _, ok := d.blobs[blob]; !ok {
			return nil, fmt.Errorf("the value names a blob it never put: %s", blob)
		}
	}
	revision := uint64(1)
	if m.Revision != nil {
		revision = *m.Revision + 1
	}
	d.values[m.Key] = plugin.StoredValue{Value: bytes.Clone(m.Value), Revision: revision}
	d.valueBlobs[m.Key] = slices.Clone(m.Blobs)
	return &plugin.PortAnswerWritten{Revision: revision}, nil
}

// setDirectoriesLocked answers the memory port operation while mu is held.
func (d *TestDemi) setDirectoriesLocked(m *plugin.PortMessageSetDirectories) (plugin.PortAnswer, error) {
	if err := plugin.CheckDirectories(m.Directories); err != nil {
		return nil, err
	}
	paths := make([]plugin.DirectoryPath, 0, len(m.Directories))
	for _, directory := range m.Directories {
		paths = append(paths, plugin.DirectoryPath{Name: directory.Name, Path: directory.Path(d.Plugin)})
	}
	d.directories = cloneDirectories(m.Directories)
	return &plugin.PortAnswerDirectories{Paths: paths}, nil
}

// readHostFilesLocked answers the memory port operation while mu is held.
func (d *TestDemi) readHostFilesLocked(m *plugin.PortMessageReadHostFiles) (plugin.PortAnswer, error) {
	if d.HostFiles == nil {
		return refused(&plugin.PortRefusalNotRunning{}), nil
	}
	files := make([]plugin.HostFile, 0, len(m.Reads))
	for _, read := range m.Reads {
		files = append(files, hostFile(d.HostFiles, read))
	}
	return &plugin.PortAnswerHostFiles{Files: files}, nil
}

// renewExposeLocked answers the memory port operation while mu is held.
func (d *TestDemi) renewExposeLocked(m *plugin.PortMessageRenewExpose) (plugin.PortAnswer, error) {
	d.liveExposesLocked()
	for i, record := range d.exposes {
		if record.ID == m.Expose {
			expiry, err := d.exposeExpiryLocked(m.Lifetime)
			if err != nil {
				return nil, err
			}
			d.exposes[i].ExpiresAt = expiry
			return &plugin.PortAnswerExpose{Expose: d.exposes[i]}, nil
		}
	}
	return exposeRefused(plugin.ExposeRefusalNotFound, "No expose "+string(m.Expose)), nil
}

// removeExposeLocked answers the memory port operation while mu is held.
func (d *TestDemi) removeExposeLocked(m *plugin.PortMessageRemoveExpose) (plugin.PortAnswer, error) {
	d.liveExposesLocked()
	for i, record := range d.exposes {
		if record.ID == m.Expose {
			d.exposes = slices.Delete(d.exposes, i, i+1)
			return &plugin.PortAnswerDone{}, nil
		}
	}
	return exposeRefused(plugin.ExposeRefusalNotFound, "No expose "+string(m.Expose)), nil
}

// readValueLocked answers the memory port operation while mu is held.
func (d *TestDemi) readValueLocked(m *plugin.PortMessageReadValue) (plugin.PortAnswer, error) {
	v, ok := d.values[m.Key]
	if !ok {
		return &plugin.PortAnswerValue{}, nil
	}
	v.Value = bytes.Clone(v.Value)
	return &plugin.PortAnswerValue{Value: &v}, nil
}

// listValuesLocked answers the memory port operation while mu is held.
func (d *TestDemi) listValuesLocked(_ *plugin.PortMessageListValues) (plugin.PortAnswer, error) {
	values := maps.Clone(d.values)
	for key, v := range values {
		v.Value = bytes.Clone(v.Value)
		values[key] = v
	}
	return &plugin.PortAnswerValues{Values: values}, nil
}

// getBlobLocked answers the memory port operation while mu is held.
func (d *TestDemi) getBlobLocked(m *plugin.PortMessageGetBlob) (plugin.PortAnswer, error) {
	value, ok := d.blobs[m.Blob]
	if !ok {
		return &plugin.PortAnswerBytes{}, nil
	}
	value = slices.Clone(value)
	return &plugin.PortAnswerBytes{Bytes: &value}, nil
}

// listExposesLocked answers the memory port operation while mu is held.
func (d *TestDemi) listExposesLocked(_ *plugin.PortMessageListExposes) (plugin.PortAnswer, error) {
	exposes := []plugin.ExposeRecord{}
	if d.ExposesAvailable {
		exposes = d.liveExposesLocked()
	}
	return &plugin.PortAnswerExposes{
		List: plugin.ExposeList{Available: d.ExposesAvailable, ListedAt: d.Now, Exposes: exposes},
	}, nil
}
