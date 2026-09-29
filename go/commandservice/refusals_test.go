package commandservice_test

import (
	"bytes"
	"context"
	"encoding/binary"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// post sends a request with the given body, and reports how the service
// answered it. A request that is accepted has its response abandoned.
func post(t *testing.T, client *commandservice.Client, method, path string, body []byte) error {
	t.Helper()
	ctx, cancel := context.WithCancel(testContext(t))
	defer cancel()
	response, err := client.Send(ctx, method, path, io.NopCloser(bytes.NewReader(body)), int64(len(body)))
	if err == nil {
		response.Body.Close()
	}
	return err
}

// framed returns document behind its length, as the metadata of a request is
// sent: it is framed as an input chunk is.
func framed(t *testing.T, document string) []byte {
	t.Helper()
	data, err := commandservice.EncodeInput([]byte(document))
	if err != nil {
		t.Fatal(err)
	}
	return data
}

// httpStatus returns the HTTP status of a request the service refused, or 0.
func httpStatus(err error) int {
	var rejected *commandservice.RejectedError
	if errors.As(err, &rejected) {
		return rejected.Status
	}
	return 0
}

func TestRequestsTheServiceRefusesBeforeRunningThem(t *testing.T) {
	server := servicetest.Start(t, operations{"noop": short})
	client := server.Client
	valid, err := commandservice.Encode(invocation("noop"))
	if err != nil {
		t.Fatal(err)
	}
	unknown, err := commandservice.Encode(invocation("missing"))
	if err != nil {
		t.Fatal(err)
	}
	oversized := make([]byte, 4)
	binary.BigEndian.PutUint32(oversized, 256*1024+1)
	for name, test := range map[string]struct {
		method, path string
		body         []byte
		want         int
	}{
		"an unknown path":                      {"POST", "/v1/resource", framed(t, `{"operation":"acquire"}`), 404},
		"the wrong method":                     {"GET", commandservice.InvokePath, nil, 404},
		"a path written with an escape":        {"GET", "/v1/%69nfo", nil, 404},
		"an unknown operation":                 {"POST", commandservice.InvokePath, framed(t, string(unknown)), 404},
		"invocation metadata that is not JSON": {"POST", commandservice.InvokePath, framed(t, `invoke`), 400},
		"invocation metadata that is invalid":  {"POST", commandservice.InvokePath, framed(t, `{"operation":"noop"}`), 400},
		"a release naming no conversation":     {"POST", commandservice.ConversationPath, framed(t, `{"operation":"release"}`), 400},
		"an unknown conversation operation":    {"POST", commandservice.ConversationPath, framed(t, `{"operation":"acquire","conversation":"one"}`), 400},
		"a release of an empty name":           {"POST", commandservice.ConversationPath, framed(t, `{"operation":"release","conversation":""}`), 400},
		"a status with a member":               {"POST", commandservice.ConversationPath, framed(t, `{"operation":"status","resource":"one"}`), 400},
		"numbers metadata with a member":       {"POST", commandservice.NumbersPath, framed(t, `{"x":1}`), 400},
		"metadata over the limit":              {"POST", commandservice.InvokePath, oversized, 400},
		"metadata cut short":                   {"POST", commandservice.InvokePath, append([]byte{0, 0, 0, 100}, "{}"...), 400},
		"no metadata":                          {"POST", commandservice.InvokePath, nil, 400},
	} {
		if got := httpStatus(post(t, client, test.method, test.path, test.body)); got != test.want {
			t.Errorf("%s: status %d, want %d", name, got, test.want)
		}
	}
	// What the service accepts still runs after all the refusals.
	if err := post(t, client, "POST", commandservice.InvokePath, append(framed(t, string(valid)), 0, 0, 0, 0)); err != nil {
		t.Errorf("a valid invocation: %v", err)
	}
	if _, err := client.Info(testContext(t)); err != nil {
		t.Errorf("the connection after the refusals: %v", err)
	}
}

// After a shutdown every new request is answered 503. The SDK's HTTP/2 server
// sends GOAWAY at once, so a request reaches the service only in the moment
// before the caller has read it.
func TestAfterAShutdownEveryNewRequestIsAnswered503(t *testing.T) {
	for name, service := range map[string]http.Handler{
		"a service that was asked to shut down":  commandservice.DrainingService(),
		"a service that stopped taking requests": commandservice.StoppedService(),
	} {
		for _, route := range []struct{ method, path string }{
			{"GET", commandservice.InfoPath},
			{"POST", commandservice.InvokePath},
			{"POST", commandservice.ConversationPath},
			{"POST", commandservice.NumbersPath},
			{"GET", "/v1/other"},
		} {
			recorder := httptest.NewRecorder()
			service.ServeHTTP(recorder, httptest.NewRequest(route.method, route.path, nil))
			if recorder.Code != http.StatusServiceUnavailable {
				t.Errorf("%s answered %s %s with %d, want 503", name, route.method, route.path, recorder.Code)
			}
		}
	}
}

// The paths of the wire are part of what a peer written in another language
// speaks: the tests name them by their literals, so that a path changed on both
// halves of the SDK fails them. A response carries no Content-Type, whose
// default net/http would sniff from the body.
func TestTheServiceAnswersOnThePathsOfTheWireWithoutAContentType(t *testing.T) {
	server := servicetest.Start(t, operations{"noop": short})
	valid, err := commandservice.Encode(invocation("noop"))
	if err != nil {
		t.Fatal(err)
	}
	status, err := commandservice.Encode(commandservice.StatusRequest{})
	if err != nil {
		t.Fatal(err)
	}
	numbers, err := commandservice.Encode(commandservice.NumbersOpen{})
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		method, path string
		body         []byte
	}{
		{"GET", "/v1/info", nil},
		{"POST", "/v1/invoke", framed(t, string(valid))},
		{"POST", "/v1/conversation", framed(t, string(status))},
		{"POST", "/v1/numbers", framed(t, string(numbers))},
		{"POST", "/v1/shutdown", nil},
	} {
		ctx, cancel := context.WithCancel(testContext(t))
		response, err := server.Client.Send(ctx, test.method, test.path, io.NopCloser(bytes.NewReader(test.body)), int64(len(test.body)))
		if err != nil {
			t.Errorf("%s %s: %v", test.method, test.path, err)
			cancel()
			continue
		}
		if contentTypes, ok := response.Header["Content-Type"]; ok {
			t.Errorf("%s %s: the response carries Content-Type %q", test.method, test.path, contentTypes)
		}
		response.Body.Close()
		cancel()
	}
}
