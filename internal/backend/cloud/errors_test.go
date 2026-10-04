package cloud_test

import (
	"errors"
	"io"
	"testing"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/webapi"
)

// Error mapping is a user-facing HTTP contract, independent of transition mechanics.
func TestCloudErrorHTTPMapping(t *testing.T) {
	cases := []struct {
		kind    cloud.ErrorKind
		code    webapi.ErrorCode
		status  int
		message string
	}{
		{cloud.Closed, webapi.ErrorCodeBackendClosing, 503, "Cloud is shutting down"},
		{
			cloud.CrashLoop,
			webapi.ErrorCodeCloudCrashLoop,
			503,
			"Cloud repeatedly failed; reset the environment to recover",
		},
		{cloud.AtCapacity, webapi.ErrorCodeCloudCapacity, 503, "Cloud capacity is currently full; retry later"},
		{cloud.Resetting, webapi.ErrorCodeCloudResetting, 409, "Another reset is in progress"},
		{cloud.Failed, webapi.ErrorCodeCloudUnavailable, 503, "unexpected EOF"},
	}
	for _, tc := range cases {
		t.Run(string(tc.code), func(t *testing.T) {
			failure := &cloud.Error{Kind: tc.kind, Err: io.ErrUnexpectedEOF}
			code, status := failure.Code()
			if code != tc.code || status != tc.status || failure.Error() != tc.message {
				t.Fatalf("HTTP failure: %s %d %s", code, status, failure.Error())
			}
			if !errors.Is(failure, io.ErrUnexpectedEOF) {
				t.Fatal("failure lost its cause")
			}
		})
	}
}
