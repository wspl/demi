package jobs

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"maps"
	"path/filepath"
	"sync"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// WorkKind distinguishes shell jobs from raw processes with independent IDs.
type WorkKind uint8

const (
	// ShellWork identifies a shell job.
	ShellWork WorkKind = iota
	// ProcessWork identifies a raw process.
	ProcessWork
)

// WorkID is a job's or raw process's ID, which the two kinds do not share.
type WorkID struct {
	// Kind selects the shell job or raw process namespace.
	Kind WorkKind
	// ID identifies work within its namespace.
	ID string
}

// Config supplies what every task of one connection shares.
// Output is a bounded queue of encoded runner frames owned by the connection.
// The caller owns Pipes, Directories and Shell and keeps them until Table.Close.
type Config struct {
	// Output receives encoded runner frames for the connection.
	Output chan<- []byte
	// Directories owns the connection job directories.
	Directories *Directories
	// Pipes transfers task input and output.
	Pipes *process.PipeClient
	// Shell starts shell jobs.
	Shell process.JobShell
	// Commands is absent where nothing makes execution contexts live.
	Commands *Commands
}

// Commands supplies the execution context and dispatcher for declared commands.
type Commands struct {
	// Dispatcher runs declared command invocations.
	Dispatcher *Dispatcher
	// Connection reaches the backend for command work.
	Connection *ConnectionHandle
	// Installation acquires the job manifest and its service leases.
	Installation *Installation
	// Paths locates the command aliases and client executable.
	Paths ContextPaths
	// Services acquires native command services.
	Services *cmdpkgs.ServiceHandle
	// Endpoint is the local endpoint command clients reach.
	Endpoint string
	// Home is the installation directory, DEMI_HOME.
	Home string
}

// TaskCommand selects shell execution or a raw process.
//
//sumtype:decl
type TaskCommand interface{ taskCommand() }

// ShellCommand runs a script with optional pipe input and output.
type ShellCommand struct {
	// Script is the shell source to execute.
	Script string
	// Stdin is the optional job input pipe.
	Stdin *runnerwire.PipeRef
	// Stdout is the optional job output pipe.
	Stdout *runnerwire.PipeRef
	// Commands pins the manifest and command context, or is nil for no declarations.
	Commands *DeclaredCommands
}

func (*ShellCommand) taskCommand() {}

// DeclaredCommands names the manifest and backend command context of a job.
type DeclaredCommands struct {
	// ManifestHash identifies the job command manifest.
	ManifestHash string
	// Context supplies the backend command authority.
	Context commandwire.CommandContext
}

// ProcessCommand runs one raw executable with its arguments.
type ProcessCommand struct {
	// Command is the executable to start.
	Command string
	// Args contains its arguments.
	Args []string
	// ProcessGroup requests ownership of the process group.
	ProcessGroup bool
}

func (*ProcessCommand) taskCommand() {}

// TaskSpec supplies a task's ID, absolute directory, environment and command.
type TaskSpec struct {
	// ID identifies the task.
	ID string
	// Cwd is the absolute working directory.
	Cwd string
	// Env supplies the task environment.
	Env map[string]string
	// Command selects shell or raw process execution.
	Command TaskCommand
}

// Table owns a connection's jobs and raw processes, including setup and IO.
// Its methods are safe for concurrent use. Its owner must call Close.
type Table struct {
	// mu protects task registrations and completion publication, never IO or waits.
	mu       sync.Mutex
	entries  map[WorkID]*taskEntry
	finished []WorkID
	changed  chan struct{}
	lifetime context.Context
	cancel   context.CancelFunc
	config   Config
	workers  sync.WaitGroup
	closing  bool
	once     sync.Once
	done     chan struct{}
}

var errTaskKilled = errors.New("task killed")

type taskEntry struct {
	lifetime  context.Context
	cancel    context.CancelCauseFunc
	input     chan process.Input
	signals   chan runnerwire.Signal
	following chan struct{}
	follow    bool
}

// NewTable creates the connection's task owner. Cancelling ctx cancels all tasks;
// Close must still join them. The connection retains ownership of config's resources.
func NewTable(ctx context.Context, config Config) *Table {
	lifetime, cancel := context.WithCancel(ctx)
	return &Table{
		entries:  make(map[WorkID]*taskEntry),
		changed:  make(chan struct{}),
		lifetime: lifetime,
		cancel:   cancel,
		config:   config,
		done:     make(chan struct{}),
	}
}

// Len returns the number of registered tasks, including completed tasks not collected.
func (t *Table) Len() int {
	t.mu.Lock()
	defer t.mu.Unlock()
	return len(t.entries)
}

// JobCount returns the number of registered shell jobs.
func (t *Table) JobCount() int {
	t.mu.Lock()
	defer t.mu.Unlock()
	count := 0
	for id := range t.entries {
		if id.Kind == ShellWork {
			count++
		}
	}
	return count
}

// Start registers the task before starting setup so subsequent input finds it.
// Task lifetime belongs to the table, not a request's context.
func (t *Table) Start(spec TaskSpec) error {
	if !filepath.IsAbs(spec.Cwd) {
		return errors.New("task cwd must be absolute")
	}
	id := WorkID{ID: spec.ID}
	switch spec.Command.(type) {
	case *ShellCommand:
		id.Kind = ShellWork
	case *ProcessCommand:
		id.Kind = ProcessWork
	default:
		return errors.New("missing task command")
	}
	spec.Env = maps.Clone(spec.Env)
	t.mu.Lock()
	if t.closing || t.lifetime.Err() != nil {
		t.mu.Unlock()
		return errors.New("task table is closed")
	}
	if t.entries[id] != nil {
		t.mu.Unlock()
		return errors.New("duplicate live task id")
	}
	lifetime, cancel := context.WithCancelCause(t.lifetime)
	entry := &taskEntry{
		lifetime:  lifetime,
		cancel:    cancel,
		input:     make(chan process.Input, 64),
		signals:   make(chan runnerwire.Signal, 16),
		following: make(chan struct{}, 1),
	}
	t.entries[id] = entry
	t.workers.Add(1)
	t.mu.Unlock()
	go t.own(id, spec, entry)
	return nil
}

// Input queues live stdin without blocking the connection. An overflowing queue
// cancels the task; bulk streams use independently flowing HTTP pipes.
func (t *Table) Input(id WorkID, bytes []byte) error {
	if len(bytes) > runnerwire.StdinChunkBytes {
		return errors.New("live stdin chunk exceeds 64 KiB")
	}
	return t.queueInput(id, process.Input{Bytes: append([]byte{}, bytes...)})
}

// EndInput queues EOF for the task's live input.
func (t *Table) EndInput(id WorkID) error {
	return t.queueInput(id, process.Input{})
}

// Follow starts or stops output beyond each stream's initial view.
func (t *Table) Follow(id WorkID, follow bool) {
	t.mu.Lock()
	entry := t.entries[id]
	if entry != nil {
		entry.follow = follow
	}
	t.mu.Unlock()
	if entry != nil {
		select {
		case entry.following <- struct{}{}:
		default:
		}
	}
}

// Signal delivers a signal without blocking; kill cancels the task.
func (t *Table) Signal(id WorkID, signal runnerwire.Signal) error {
	t.mu.Lock()
	entry := t.entries[id]
	t.mu.Unlock()
	if entry == nil {
		return nil
	}
	if signal == runnerwire.SignalKill {
		entry.cancel(errTaskKilled)
		return nil
	}
	select {
	case <-entry.lifetime.Done():
		return nil
	case entry.signals <- signal:
		return nil
	default:
		return errors.New("signal queue unavailable: no available capacity")
	}
}

// Finished waits for the next task after its result has been sent, removing its
// entry. The boolean is false when no task runs; an interrupted wait returns an error.
func (t *Table) Finished(ctx context.Context) (WorkID, bool, error) {
	for {
		t.mu.Lock()
		if len(t.finished) > 0 {
			id := t.finished[0]
			t.finished = t.finished[1:]
			delete(t.entries, id)
			t.mu.Unlock()
			return id, true, nil
		}
		empty := len(t.entries) == 0
		changed := t.changed
		t.mu.Unlock()
		if empty {
			return WorkID{}, false, nil
		}
		select {
		case <-ctx.Done():
			return WorkID{}, false, ctx.Err()
		case <-changed:
		}
	}
}

// Close cancels and joins every task, suppressing further results. Cancellation
// of ctx still initiates shutdown; a later Close can finish waiting. It is idempotent.
func (t *Table) Close(ctx context.Context) error {
	t.once.Do(func() {
		t.mu.Lock()
		t.closing = true
		t.mu.Unlock()
		t.cancel()
		go func() {
			t.workers.Wait()
			t.mu.Lock()
			clear(t.entries)
			t.finished = nil
			old := t.changed
			t.changed = make(chan struct{})
			t.mu.Unlock()
			close(old)
			close(t.done)
		}()
	})
	select {
	case <-t.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// FailureExit encodes the end of work that could not run, or failed before its
// status was known: no exit status and reason as its spawn error.
func FailureExit(work WorkID, reason string) ([]byte, error) {
	failure := &runnerwire.SpawnError{Kind: runnerwire.SpawnErrorKindOther, Detail: &reason}
	if work.Kind == ShellWork {
		return runnerwire.Encode(
			&runnerwire.JobExit{JobID: work.ID, SpawnError: failure, Files: []runnerwire.JobFileChange{}},
		)
	}
	return runnerwire.Encode(&runnerwire.SpawnExit{SpawnID: work.ID, SpawnError: failure})
}

// queueInput applies the connection's bounded live-input policy without waiting.
func (t *Table) queueInput(id WorkID, input process.Input) error {
	t.mu.Lock()
	entry := t.entries[id]
	t.mu.Unlock()
	if entry == nil {
		return nil
	}
	select {
	case <-entry.lifetime.Done():
		return nil
	default:
	}
	select {
	case entry.input <- input:
		return nil
	default:
		entry.cancel(nil)
		return errors.New("live stdin buffer is full")
	}
}

// own contains one task's failure and publishes completion only after its result.
func (t *Table) own(id WorkID, spec TaskSpec, entry *taskEntry) {
	defer t.workers.Done()
	defer entry.cancel(nil)
	result, err := func() (frame []byte, err error) {
		defer func() {
			if recovered := recover(); recovered != nil {
				err = fmt.Errorf("task owner panicked")
			}
		}()
		return t.run(spec, entry)
	}()
	if err != nil {
		result, err = FailureExit(id, err.Error())
	}
	if err == nil {
		select {
		case <-t.lifetime.Done():
		case t.config.Output <- result:
		}
	} else {
		slog.Warn("task terminal encoding failed", "error", err)
	}
	t.mu.Lock()
	t.finished = append(t.finished, id)
	old := t.changed
	t.changed = make(chan struct{})
	t.mu.Unlock()
	close(old)
}
