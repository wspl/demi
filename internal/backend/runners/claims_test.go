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
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
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
	for _, spelling := range []string{printed, " " + strings.ReplaceAll(strings.ToLower(printed), "-", " ") + " ", strings.NewReplacer("0", "o", "1", "l").Replace(printed)} {
		parsed, ok := runners.ParseClaimCode(spelling)
		if !ok || parsed != code {
			t.Fatalf("did not read %q", spelling)
		}
	}
	for _, invalid := range []string{"", "AAAA-BBBB", "NOPE-NOPE", printed[:len(printed)-1], strings.Repeat("U", 26), strings.Repeat("0", 25) + "1"} {
		if _, ok := runners.ParseClaimCode(invalid); ok {
			t.Fatalf("accepted %q", invalid)
		}
	}
}

func TestClaimOwnershipAndAbandonment(t *testing.T) {
	for _, ending := range []string{"bound", "withdrawn", "runner-left", "claim-left", "shutdown", "replaced", "cancelled"} {
		t.Run(ending, func(t *testing.T) {
			claims := runners.NewPendingClaims(10)
			defer claims.Close()
			code := runners.GenerateClaimCode()
			wait := claims.Register(code, runnerwire.RunnerInfo{Name: "fixture"})
			defer wait.Release()
			if ending == "withdrawn" {
				claims.Withdraw(code)
			}
			if ending == "shutdown" {
				claims.Close()
			}
			if ending == "replaced" {
				next := claims.Register(code, runnerwire.RunnerInfo{Name: "next"})
				defer next.Release()
			}
			if ending == "withdrawn" || ending == "shutdown" || ending == "replaced" {
				grant, err := wait.Wait(t.Context())
				if err != nil || grant != nil {
					t.Fatalf("abandoned wait: %v %v", grant, err)
				}
				if ending == "shutdown" && claims.Register(code, runnerwire.RunnerInfo{}) != nil {
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
			answer := runner.Grant(database.DeviceRecord{ID: "device"}, token)
			defer answer.Release()
			runner.Release() // Does not withdraw a transferred grant.
			if ending == "bound" {
				grant, err := wait.Wait(t.Context())
				if err != nil || grant == nil || grant.Device.ID != "device" || grant.Token != token {
					t.Fatal("wrong grant", err)
				}
				defer grant.Release()
				grant.Bound(webapi.DeviceDTO{ID: "device", Online: true})
				grant.Release()
			}
			device, err := answer.Wait(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			if ending == "bound" {
				if device == nil || !device.Online {
					t.Fatal("missing bound answer")
				}
			} else if device != nil {
				t.Fatal("departed runner bound")
			}
		})
	}
}

func TestClaimGrantRacesDeparture(t *testing.T) {
	// Exercise both channel handoffs without timing assumptions or orphaned owners.
	for range 100 {
		claims := runners.NewPendingClaims(10)
		code := runners.GenerateClaimCode()
		wait := claims.Register(code, runnerwire.RunnerInfo{})
		runner := claims.Take(code)
		var workers sync.WaitGroup
		workers.Go(func() { wait.Release() })
		answer := runner.Grant(database.DeviceRecord{}, runners.NewDeviceToken())
		workers.Wait()
		device, err := answer.Wait(t.Context())
		if device != nil || err != nil {
			t.Fatalf("departure answer %v %v", device, err)
		}
		answer.Release()
		runner.Release()
		claims.Close()
	}
}
