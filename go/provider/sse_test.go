package provider_test

import (
	"context"
	"errors"
	"io"
	"net/http"
	"reflect"
	"strings"
	"testing"
	"testing/iotest"

	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/providertest"
)

// Tests use local scripts and event waits; each takes well under one second.
func TestSSEFraming(t *testing.T) {
	for _, tc := range []struct {
		body string
		want []string
	}{
		{"event: delta\r\ndata: {\"a\":1}\r\n\r\n", []string{`{"a":1}`}},
		{"data: line one\ndata: line two\n\n", []string{"line one\nline two"}},
		{"data: x\n\ndata: [DONE]\n\n", []string{"x", "[DONE]"}},
		{"", nil},
		{"data: x", []string{"x"}},
		{"data: x\n", []string{"x"}},
		{"data: héllo\n\n", []string{"héllo"}},
		{": keep-alive\n\nevent: ping\n\ndata:\n\ndata: x\n\n", []string{"x"}},
		{"\ufeffdata: first\r\rdata: second\r\r", []string{"first", "second"}},
	} {
		var got []string
		for data, err := range provider.SSEData(iotest.OneByteReader(strings.NewReader(tc.body))) {
			if err != nil {
				t.Fatal(err)
			}
			got = append(got, data)
		}
		if !reflect.DeepEqual(got, tc.want) {
			t.Fatalf("%q: got %q, want %q", tc.body, got, tc.want)
		}
	}
}
func TestSSEBrokenBodyPreservesEarlierFrames(t *testing.T) {
	vendor := providertest.NewMockVendor(t, providertest.MockResponse{Chunks: []string{"data: first\n\n"}, Ending: providertest.Broken})
	response, err := vendor.Server.Client().Get(vendor.Server.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	var frames []string
	var failure error
	for data, err := range provider.SSEData(response.Body) {
		if err != nil {
			failure = err
			break
		}
		frames = append(frames, data)
	}
	if !reflect.DeepEqual(frames, []string{"first"}) || !errors.Is(failure, io.ErrUnexpectedEOF) {
		t.Fatalf("frames=%q error=%v", frames, failure)
	}
}
func TestSSECancellationClosesHeldVendorResponse(t *testing.T) {
	vendor := providertest.NewMockVendor(t, providertest.MockResponse{Chunks: []string{"data: first\n\n"}, Ending: providertest.Open})
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	request, err := http.NewRequestWithContext(ctx, "GET", vendor.Server.URL, nil)
	if err != nil {
		t.Fatal(err)
	}
	response, err := vendor.Server.Client().Do(request)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	for _, err := range provider.SSEData(response.Body) {
		if err != nil {
			t.Fatal(err)
		}
		cancel()
		break
	}
	vendor.WaitDisconnect(t)
}
func TestSSERejectsInvalidUTF8(t *testing.T) {
	var failure error
	for _, err := range provider.SSEData(strings.NewReader("data: \xff\n\n")) {
		failure = err
	}
	if failure == nil {
		t.Fatal("invalid UTF-8 accepted")
	}
}
