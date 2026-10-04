package runners_test

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/webapiproto"
)

func TestClaimAttemptWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		claims := runners.NewPendingClaims(2)
		defer claims.Close()
		if !claims.Attempt("ana") {
			t.Fatal("first attempt refused")
		}
		time.Sleep(30 * time.Second) // Virtual time advances the claim window.
		if !claims.Attempt("ana") || claims.Attempt("ana") || !claims.Attempt("ben") {
			t.Fatal("per-user allowance changed")
		}
		time.Sleep(30 * time.Second)
		if !claims.Attempt("ana") || claims.Attempt("ana") {
			t.Fatal("window did not expire exactly at sixty seconds")
		}
	})
}

func TestClaimCodeSpellings(t *testing.T) {
	code := runners.GenerateClaimCode()
	printed := code.Printed()
	if len(printed) != 32 {
		t.Fatalf("printed %q", printed)
	}
	for _, group := range strings.Split(printed, "-") {
		if len(group) < 1 || len(group) > 4 {
			t.Fatalf("group %q", group)
		}
	}
	if strings.Trim(printed, "0123456789ABCDEFGHJKMNPQRSTVWXYZ-") != "" {
		t.Fatal("unexpected alphabet", printed)
	}
	for _, spelling := range []string{
		printed,
		" " + strings.ReplaceAll(strings.ToLower(printed), "-", " ") + " ",
		strings.NewReplacer("0", "o", "1", "l").Replace(printed),
	} {
		parsed, ok := runners.ParseClaimCode(spelling)
		if !ok || parsed != code {
			t.Fatalf("did not read %q", spelling)
		}
	}
	for _, invalid := range []string{
		"",
		"AAAA-BBBB",
		"NOPE-NOPE",
		printed[:len(printed)-1],
		strings.Repeat("U", 26),
		strings.Repeat("0", 25) + "1",
	} {
		if _, ok := runners.ParseClaimCode(invalid); ok {
			t.Fatalf("accepted %q", invalid)
		}
	}
}

func TestClaimOwnershipAndAbandonment(t *testing.T) {
	for _, ending := range []string{
		"bound",
		"withdrawn",
		"runner-left",
		"claim-left",
		"shutdown",
		"replaced",
		"cancelled",
	} {
		t.Run(ending, func(t *testing.T) {
			type granted struct {
				device webapiproto.DeviceDTO
				err    error
			}
			claims := runners.NewPendingClaims(10)
			defer claims.Close()
			code := runners.GenerateClaimCode()
			wait := claims.Register(code, runnerproto.Info{Name: "fixture"})
			defer wait.Release()
			if ending == "withdrawn" {
				claims.Withdraw(code)
			}
			if ending == "shutdown" {
				claims.Close()
			}
			if ending == "replaced" {
				next := claims.Register(code, runnerproto.Info{Name: "next"})
				defer next.Release()
			}
			if ending == "withdrawn" || ending == "shutdown" || ending == "replaced" {
				grant, err := wait.Wait(t.Context())
				if err != nil || grant != nil {
					t.Fatalf("abandoned wait: %v %v", grant, err)
				}
				if ending == "shutdown" && claims.Register(code, runnerproto.Info{}) != nil {
					t.Fatal("registered after shutdown")
				}
				return
			}
			runner := claims.Take(code)
			if runner == nil || runner.Runner.Name != "fixture" || claims.Take(code) != nil {
				t.Fatal("claim not exclusive")
			}
			defer runner.Release()
			if ending == "claim-left" {
				runner.Release()
				grant, err := wait.Wait(t.Context())
				if grant != nil || err != nil {
					t.Fatal("abandoned claim not completed")
				}
				return
			}
			if ending == "cancelled" {
				ctx, cancel := context.WithCancel(t.Context())
				cancel()
				if _, err := wait.Wait(ctx); !errors.Is(err, context.Canceled) {
					t.Fatalf("cancel wait: %v", err)
				}
			}
			if ending == "runner-left" || ending == "cancelled" {
				wait.Release()
			}
			token := runners.NewDeviceToken()
			answered := make(chan granted, 1)
			go func() {
				device, err := runner.Grant(t.Context(), database.DeviceRecord{ID: "device"}, token)
				answered <- granted{device: device, err: err}
			}()
			if ending == "bound" {
				grant, err := wait.Wait(t.Context())
				if err != nil || grant == nil || grant.Device.ID != "device" || grant.Token != token {
					t.Fatal("wrong grant", err)
				}
				runner.Release() // Does not withdraw a transferred grant.
				defer grant.Release()
				grant.Bound(webapiproto.DeviceDTO{ID: "device", Online: true})
				grant.Release()
			}
			result := <-answered
			runner.Release()
			if ending == "bound" {
				if result.err != nil || !result.device.Online {
					t.Fatal("missing bound answer", result.err)
				}
			} else if !errors.Is(result.err, runners.ErrRunnerLeft) {
				t.Fatal("departed runner bound", result.err)
			}
		})
	}
}

func TestClaimGrantRacesDeparture(t *testing.T) {
	// Exercise both channel handoffs without timing assumptions or orphaned owners.
	for range 100 {
		claims := runners.NewPendingClaims(10)
		code := runners.GenerateClaimCode()
		wait := claims.Register(code, runnerproto.Info{})
		runner := claims.Take(code)
		var workers sync.WaitGroup
		workers.Go(func() { wait.Release() })
		_, err := runner.Grant(t.Context(), database.DeviceRecord{}, runners.NewDeviceToken())
		workers.Wait()
		if !errors.Is(err, runners.ErrRunnerLeft) {
			t.Fatalf("departure answer %v", err)
		}
		runner.Release()
		claims.Close()
	}
}
