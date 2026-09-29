package backendtest_test

import (
	"net/http"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/controlproto"
)

// Cost: one backend, about a second.
func TestTheControlClientSpeaksEveryOperationOfTheControlSocket(t *testing.T) {
	t.Parallel()
	b, _ := backendtest.New(t, backendtest.WithMail()).StartSetUp()
	control := b.Control

	// The clock stands still where the control puts it, and the backend reads it.
	at := time.Date(2030, 1, 2, 3, 4, 5, 0, time.UTC)
	control.SetClock(at)
	if moved := control.AdvanceClock(time.Minute); !moved.Equal(at.Add(time.Minute)) {
		t.Fatalf("the clock reads %v", moved)
	}
	// The master's session ended with the years the clock jumped.
	master := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	created := b.Post("/api/users", master, backendtest.Map{"email": "clock@example.test", "password": "clock-pass-1", "role": "user"}).Expect(http.StatusCreated)
	if got := instant(t, created.At("user.createdAt")); !got.Equal(at.Add(time.Minute)) {
		t.Fatalf("an account is created at %v", got)
	}

	// An account added without a route signs in.
	added := control.AddUser("added@example.test", "added-pass-1", "user")
	if signedIn := b.Login("added@example.test", "added-pass-1"); signedIn.User.ID != added {
		t.Fatalf("the added account signed in as %s, not %s", signedIn.User.ID, added)
	}

	// Every hold target is taken, waited at and released; a lease of the file gate
	// is entered, waited behind and released.
	for _, target := range []string{
		controlproto.HoldCommits, controlproto.HoldHelloTokenLookup, controlproto.HoldHelloBind,
		controlproto.HoldSyncSnapshot, controlproto.HoldSyncChanges,
	} {
		hold := control.Hold(target)
		hold.Wait(0)
		hold.Release()
	}
	lease := control.EnterGate(added, convFirst, "demand")
	control.WaitGateEntrants(added, convFirst, 0)
	lease.Release()

	control.RunRetention(added)
	control.ObjectCounts()
	if mail := control.Mail(); len(mail) != 0 {
		t.Fatalf("the mailbox holds %v", mail)
	}
	control.FailMail(true)
	control.FailMail(false)
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestAHoldAndALeaseEndWhenTheirControlConnectionCloses(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	b.CreateConversation(master, convFirst)
	laptop := b.Pair(master, "laptop")
	switchTarget := func() *backendtest.Answer {
		return b.Patch("/api/conversations/"+convFirst, master, backendtest.Map{"target": deviceTarget(laptop, laptop.Home())})
	}

	// A hold stops a page's channel before its snapshot; once the connection that
	// took the hold closes, the snapshot comes.
	holding := b.NewControl()
	hold := holding.Hold(controlproto.HoldSyncSnapshot)
	page := b.Sync(master)
	hold.Wait(1)
	holding.Close()
	page.Snapshot()

	// A lease of the conversation's file gate refuses a switch of its target,
	// which needs the gate to itself; once the connection that holds the lease
	// closes, the switch goes through.
	leasing := b.NewControl()
	leasing.EnterGate(master.User.ID, convFirst, "demand")
	wantRefusal(t, switchTarget(), http.StatusConflict, "turn_in_flight", "a switch while the lease is held")
	leasing.Close()
	backendtest.Eventually(t, "the switch goes through once the lease's connection ended", func() bool {
		return switchTarget().Status == http.StatusOK
	})
	b.Stop()
}

// Cost: one backend, about a second.
func TestAReleaseEndsTheWaitsAtItsHoldAndFreesTheFlow(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	control := b.NewControl()
	hold := control.Hold(controlproto.HoldSyncSnapshot)
	waited := make(chan error, 1)
	go func() {
		// Nothing brings nine pages to the hold, so this wait ends only with the
		// release.
		waited <- hold.TryWait(9)
	}()
	page := b.Sync(master)
	hold.Wait(1)
	hold.Release()
	select {
	case err := <-waited:
		if err == nil {
			t.Fatal("a wait for nine pages succeeded")
		}
	case <-time.After(backendtest.Patience):
		t.Fatal("the release did not end the wait")
	}
	page.Snapshot()
	b.Stop()
}
