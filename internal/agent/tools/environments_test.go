package tools

import (
	"context"
	"errors"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

// ownedEnvironment exposes handle ownership and records cleanup for node lifecycle tests.
type ownedEnvironment struct {
	host.ShellEnvironment
	commands  []types.CommandID
	shells    []types.ShellID
	disposals atomic.Int32
	dispose   func(context.Context) error
}

func (e *ownedEnvironment) OwnsCommand(command types.CommandID) bool {
	for _, id := range e.commands {
		if id == command {
			return true
		}
	}
	return false
}

func (e *ownedEnvironment) OwnsShell(shell types.ShellID) bool {
	for _, id := range e.shells {
		if id == shell {
			return true
		}
	}
	return false
}

func (e *ownedEnvironment) DisposeAll(ctx context.Context) error {
	e.disposals.Add(1)
	if e.dispose != nil {
		return e.dispose(ctx)
	}
	return nil
}

func (e *ownedEnvironment) PageViews() []host.PageView {
	var views []host.PageView
	for _, id := range e.commands {
		views = append(views, host.PageView{CommandID: id})
	}
	return views
}

func TestConcurrentCallsMakeOneEnvironment(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var environments Environments
		owner := &ownedEnvironment{}
		gate := make(chan struct{})
		started := make(chan struct{})
		var made atomic.Int32
		create := func(context.Context) (host.ShellEnvironment, error) {
			made.Add(1)
			close(started)
			<-gate
			return owner, nil
		}
		type answer struct {
			slot *environmentSlot
			err  error
		}
		results := make(chan answer, 2)
		for range 2 {
			go func() {
				slot, _, err := environments.resolve(t.Context(), "a", nil, nil, create)
				results <- answer{slot, err}
			}()
		}
		<-started
		synctest.Wait()
		close(gate)
		first, second := <-results, <-results
		if first.err != nil || second.err != nil || first.slot != second.slot || made.Load() != 1 {
			t.Fatalf("creation not shared: %v %v count %d", first.err, second.err, made.Load())
		}
		if err := environments.Dispose(t.Context()); err != nil {
			t.Fatal(err)
		}
		if owner.disposals.Load() != 1 {
			t.Fatal("environment not released once")
		}
	})
}

func TestHandleBelongsToItsHost(t *testing.T) {
	var environments Environments
	defer func() {
		if err := environments.Dispose(context.WithoutCancel(t.Context())); err != nil {
			t.Error(err)
		}
	}()
	command := types.CommandID("cmd-a")
	shell := types.ShellID("shell-a")
	owner := &ownedEnvironment{commands: []types.CommandID{command}, shells: []types.ShellID{shell}}
	create := func(context.Context) (host.ShellEnvironment, error) { return owner, nil }
	if _, _, err := environments.resolve(t.Context(), "a", nil, nil, create); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct {
		name    string
		shell   *types.ShellID
		command *types.CommandID
		want    string
	}{
		{"command", nil, &command, `Shell handle "cmd-a" belongs to a different Host`},
		{"shell", &shell, nil, `Shell handle "shell-a" belongs to a different Host`},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			_, _, err := environments.resolve(
				t.Context(),
				"b",
				scenario.shell,
				scenario.command,
				func(context.Context) (host.ShellEnvironment, error) { return &ownedEnvironment{}, nil },
			)
			if err == nil || err.Error() != scenario.want {
				t.Fatalf("got %v want %s", err, scenario.want)
			}
		})
	}
	if _, _, err := environments.resolve(
		t.Context(),
		"a",
		nil,
		&command,
		func(context.Context) (host.ShellEnvironment, error) {
			t.Fatal("created twice")
			return nil, nil
		},
	); err != nil {
		t.Fatal(err)
	}
	if environments.Owning(command) != owner || len(environments.PageViews()) != 1 {
		t.Fatal("lost owned command")
	}
	var twice Environments
	defer func() {
		if err := twice.Dispose(context.WithoutCancel(t.Context())); err != nil {
			t.Error(err)
		}
	}()
	for _, key := range []host.Key{"a", "b"} {
		if _, _, err := twice.resolve(t.Context(), key, nil, nil, func(context.Context) (host.ShellEnvironment, error) {
			return &ownedEnvironment{commands: []types.CommandID{command}, shells: []types.ShellID{shell}}, nil
		}); err != nil {
			t.Fatal(err)
		}
	}
	for _, scenario := range []struct {
		shell   *types.ShellID
		command *types.CommandID
		want    string
	}{
		{nil, &command, `Command id "cmd-a" is not unique in this session`},
		{&shell, nil, `Shell id "shell-a" is not unique in this session`},
	} {
		_, _, err := twice.resolve(t.Context(), "a", scenario.shell, scenario.command, create)
		if err == nil || err.Error() != scenario.want {
			t.Fatalf("got %v want %s", err, scenario.want)
		}
	}
}

func TestDisposeEndsExistingAndConcurrentCreation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var environments Environments
		first, late := &ownedEnvironment{}, &ownedEnvironment{}
		if _, _, err := environments.resolve(
			t.Context(),
			"a",
			nil,
			nil,
			func(context.Context) (host.ShellEnvironment, error) { return first, nil },
		); err != nil {
			t.Fatal(err)
		}
		gate, started := make(chan struct{}), make(chan struct{})
		made := make(chan error, 1)
		go func() {
			_, _, err := environments.resolve(
				t.Context(),
				"b",
				nil,
				nil,
				func(context.Context) (host.ShellEnvironment, error) {
					close(started)
					<-gate
					return late, nil
				},
			)
			made <- err
		}()
		<-started
		disposed := make(chan error, 1)
		go func() { disposed <- environments.Dispose(t.Context()) }()
		synctest.Wait()
		premature := false
		select {
		case err := <-disposed:
			premature = true
			t.Errorf("dispose returned before creation joined: %v", err)
		default:
		}
		close(gate)
		if err := <-made; err == nil {
			t.Fatal("late environment was admitted")
		}
		if !premature {
			if err := <-disposed; err != nil {
				t.Fatal(err)
			}
		}
		if first.disposals.Load() != 1 || late.disposals.Load() != 1 {
			t.Fatal("cleanup missing or duplicated")
		}
		if _, _, err := environments.resolve(
			t.Context(),
			"a",
			nil,
			nil,
			func(context.Context) (host.ShellEnvironment, error) {
				t.Fatal("created after disposal")
				return nil, nil
			},
		); err == nil {
			t.Fatal("admitted after disposal")
		}
	})
}

func TestSeventhIdenticalExecSuppressed(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var slot environmentSlot
		for range 6 {
			if _, suppressed := slot.repeated("make"); suppressed {
				t.Fatal("suppressed too early")
			}
		}
		for _, count := range []uint32{7, 8} {
			got, suppressed := slot.repeated("make")
			if !suppressed || !got.IsError {
				t.Fatal("repeat not suppressed")
			}
			view, ok := got.View.(*types.RepeatedShellExec)
			if !ok || view.Count != count || view.Script != "make" {
				t.Fatalf("wrong view: %+v", got.View)
			}
		}
		if _, suppressed := slot.repeated("make test"); suppressed {
			t.Fatal("different script suppressed")
		}
		for range 5 {
			if _, suppressed := slot.repeated("make"); suppressed {
				t.Fatal("count not reset")
			}
		}
		time.Sleep(60*time.Second + time.Millisecond) // synctest advances virtual time.
		if _, suppressed := slot.repeated("make"); suppressed {
			t.Fatal("repeat window did not expire")
		}
	})
}

func TestEndAllRecreatesAndCleanupErrorsAreReturned(t *testing.T) {
	var environments Environments
	failure := errors.New("cleanup failed")
	first := &ownedEnvironment{dispose: func(context.Context) error { return failure }}
	if _, _, err := environments.resolve(
		t.Context(),
		"a",
		nil,
		nil,
		func(context.Context) (host.ShellEnvironment, error) { return first, nil },
	); err != nil {
		t.Fatal(err)
	}
	if err := environments.EndAll(t.Context()); !errors.Is(err, failure) {
		t.Fatalf("cleanup error lost: %v", err)
	}
	next := &ownedEnvironment{}
	if _, got, err := environments.resolve(
		t.Context(),
		"a",
		nil,
		nil,
		func(context.Context) (host.ShellEnvironment, error) { return next, nil },
	); err != nil ||
		got != next {
		t.Fatalf("did not create fresh environment: %v", err)
	}
	if err := environments.Dispose(t.Context()); err != nil {
		t.Fatal(err)
	}
	if first.disposals.Load() != 1 || next.disposals.Load() != 1 {
		t.Fatal("cleanup missing or duplicated")
	}
}

func TestDisposeJoinsConcurrentEndAll(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var environments Environments
		closing, finish := make(chan struct{}), make(chan struct{})
		owner := &ownedEnvironment{dispose: func(context.Context) error {
			close(closing)
			<-finish
			return nil
		}}
		if _, _, err := environments.resolve(
			t.Context(),
			"a",
			nil,
			nil,
			func(context.Context) (host.ShellEnvironment, error) { return owner, nil },
		); err != nil {
			t.Fatal(err)
		}
		ended := make(chan error, 1)
		go func() { ended <- environments.EndAll(t.Context()) }()
		<-closing
		disposed := make(chan error, 1)
		go func() { disposed <- environments.Dispose(t.Context()) }()
		synctest.Wait()
		premature := false
		select {
		case err := <-disposed:
			premature = true
			t.Errorf("dispose returned before cleanup: %v", err)
		default:
		}
		close(finish)
		if err := <-ended; err != nil {
			t.Fatal(err)
		}
		if !premature {
			if err := <-disposed; err != nil {
				t.Fatal(err)
			}
		}
		if owner.disposals.Load() != 1 {
			t.Fatal("closed more than once")
		}
	})
}

func TestCancelledWaiterDoesNotCancelSharedCreation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var environments Environments
		owner := &ownedEnvironment{}
		started, finish := make(chan struct{}), make(chan struct{})
		created := make(chan error, 1)
		go func() {
			_, _, err := environments.resolve(
				t.Context(),
				"a",
				nil,
				nil,
				func(context.Context) (host.ShellEnvironment, error) {
					close(started)
					<-finish
					return owner, nil
				},
			)
			created <- err
		}()
		<-started
		ctx, cancel := context.WithCancel(t.Context())
		waited := make(chan error, 1)
		go func() {
			_, _, err := environments.resolve(
				ctx,
				"a",
				nil,
				nil,
				func(context.Context) (host.ShellEnvironment, error) { return nil, errors.New("created twice") },
			)
			waited <- err
		}()
		synctest.Wait()
		cancel()
		waitErr := <-waited
		close(finish)
		createErr := <-created
		closeErr := environments.Dispose(t.Context())
		if !errors.Is(waitErr, context.Canceled) || createErr != nil || closeErr != nil || owner.disposals.Load() != 1 {
			t.Fatalf("wait %v create %v close %v", waitErr, createErr, closeErr)
		}
	})
}
