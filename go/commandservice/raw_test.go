package commandservice_test

import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/commandservice"
)

// The tests of this file speak HTTP/2 by hand, frame by frame, as a peer that
// is not the SDK does, to see what the SDK puts on the wire. The standard
// library has no public HTTP/2 framer.

// clientPreface is what a client sends first (RFC 9113 § 3.4).
const clientPreface = "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"

// The kinds of frame the tests use (RFC 9113 § 6).
const (
	frameData         = 0
	frameSettings     = 4
	frameWindowUpdate = 8
)

// The settings the tests read (RFC 9113 § 6.5.2).
const (
	settingMaxConcurrentStreams = 3
	settingInitialWindowSize    = 4
	settingMaxHeaderListSize    = 6
)

type h2Frame struct {
	kind    byte
	flags   byte
	stream  uint32
	payload []byte
}

// bytes returns the frame as it goes on the wire: three bytes of length, the
// kind, the flags, the stream, and the payload.
func (f h2Frame) bytes() []byte {
	data := make([]byte, 9, 9+len(f.payload))
	data[0], data[1], data[2] = byte(len(f.payload)>>16), byte(len(f.payload)>>8), byte(len(f.payload))
	data[3] = f.kind
	data[4] = f.flags
	binary.BigEndian.PutUint32(data[5:], f.stream)
	return append(data, f.payload...)
}

func readH2Frame(r io.Reader) (h2Frame, error) {
	var header [9]byte
	if _, err := io.ReadFull(r, header[:]); err != nil {
		return h2Frame{}, err
	}
	length := int(header[0])<<16 | int(header[1])<<8 | int(header[2])
	frame := h2Frame{kind: header[3], flags: header[4], stream: binary.BigEndian.Uint32(header[5:]) &^ (1 << 31)}
	frame.payload = make([]byte, length)
	_, err := io.ReadFull(r, frame.payload)
	return frame, err
}

// The limits a peer states in its first frames: its settings by their
// identifiers, and the window it gives the connection on top of the 65,535
// bytes every connection starts with.
type h2Limits struct {
	settings map[uint16]uint32
	window   uint32
}

// readLimits reads frames from r until it has the peer's settings and the
// window update of its connection.
func readLimits(t *testing.T, conn net.Conn) h2Limits {
	t.Helper()
	// The read of a frame that never comes ends with the deadline.
	if err := conn.SetReadDeadline(time.Now().Add(hang)); err != nil {
		t.Fatal(err)
	}
	limits := h2Limits{settings: map[uint16]uint32{}}
	seenSettings, seenWindow := false, false
	for !seenSettings || !seenWindow {
		frame, err := readH2Frame(conn)
		if err != nil {
			t.Fatal(err)
		}
		switch {
		case frame.kind == frameSettings && frame.flags&1 == 0:
			seenSettings = true
			for entry := frame.payload; len(entry) >= 6; entry = entry[6:] {
				limits.settings[binary.BigEndian.Uint16(entry)] = binary.BigEndian.Uint32(entry[2:])
			}
		case frame.kind == frameWindowUpdate && frame.stream == 0:
			seenWindow = true
			limits.window = binary.BigEndian.Uint32(frame.payload)
		}
	}
	return limits
}

// Both halves of the wire state the same limits: a stream's window of 64 KiB, a
// connection window of 2^31-1 (the largest HTTP/2 allows), and a header list of
// 16 KiB, so that no connection is a constraint of any stream, and none holds an
// unbounded header.
func TestTheServiceStatesTheWiresLimits(t *testing.T) {
	peer, serviceEnd := net.Pipe()
	ctx, stop := context.WithCancel(testContext(t))
	served := make(chan error, 1)
	go func() { served <- commandservice.Serve(ctx, serviceEnd, operations{"noop": short}) }()
	t.Cleanup(func() {
		stop()
		<-served
	})
	go func() {
		// A client's preface, and its settings, which are empty.
		_, _ = peer.Write([]byte(clientPreface))
		_, _ = peer.Write(h2Frame{kind: frameSettings}.bytes())
	}()

	limits := readLimits(t, peer)
	if got := limits.settings[settingMaxHeaderListSize]; got != 16384 {
		t.Errorf("the header list is %d bytes, want 16384", got)
	}
	if got := limits.settings[settingInitialWindowSize]; got != 65536 {
		t.Errorf("the window of a stream is %d bytes, want 65536", got)
	}
	if got := limits.settings[settingMaxConcurrentStreams]; got != 2147483647 {
		t.Errorf("the streams at once are %d, want 2147483647", got)
	}
	if got := 65535 + limits.window; got != 2147483647 {
		t.Errorf("the window of the connection is %d bytes, want 2147483647", got)
	}
}

func TestTheClientStatesTheWiresLimits(t *testing.T) {
	peer, clientEnd := net.Pipe()
	connected := make(chan *commandservice.Client, 1)
	go func() {
		client, err := commandservice.Connect(testContext(t), clientEnd)
		if err != nil {
			t.Error(err)
		}
		connected <- client
	}()
	// The preface comes first; the client's settings and its window follow.
	preface := make([]byte, len(clientPreface))
	if _, err := io.ReadFull(peer, preface); err != nil || string(preface) != clientPreface {
		t.Fatalf("the client began with %q, %v", preface, err)
	}
	limits := readLimits(t, peer)
	// A service's own settings let the client go on.
	if _, err := peer.Write(h2Frame{kind: frameSettings}.bytes()); err != nil {
		t.Fatal(err)
	}
	client := <-connected
	if client != nil {
		t.Cleanup(func() { client.Close() })
	}

	if got := limits.settings[settingMaxHeaderListSize]; got != 16384 {
		t.Errorf("the header list is %d bytes, want 16384", got)
	}
	if got := limits.settings[settingInitialWindowSize]; got != 65536 {
		t.Errorf("the window of a stream is %d bytes, want 65536", got)
	}
	if got := 65535 + limits.window; got != 2147483647 {
		t.Errorf("the window of the connection is %d bytes, want 2147483647", got)
	}
}

// A peer that breaks the protocol after its handshake has failed the
// connection: the HTTP/2 server sends GOAWAY and closes it, and Serve returns
// the failure, where it returns nothing for a peer that closes.
func TestAPeerThatBreaksTheProtocolAfterItsHandshakeFailsTheService(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		peer, serviceEnd := net.Pipe()
		// What the service says goes nowhere; the server waits a second after its
		// GOAWAY for the peer to close, and the fake clock spends the second.
		go func() { _, _ = io.Copy(io.Discard, peer) }()
		served := make(chan error, 1)
		go func() { served <- commandservice.Serve(testContext(t), serviceEnd, operations{"noop": short}) }()

		_, _ = peer.Write([]byte(clientPreface))
		_, _ = peer.Write(h2Frame{kind: frameSettings}.bytes())
		// Data belongs to a stream, and stream 0 is the connection's own.
		_, _ = peer.Write(h2Frame{kind: frameData, payload: []byte("x")}.bytes())

		err := <-served
		if err == nil || errors.Is(err, commandservice.ErrHandshakeTimeout) || errors.Is(err, commandservice.ErrCancelled) {
			t.Errorf("Serve returned %v, want the failure of the connection", err)
		}
		// The service closed its end of the connection.
		if err := peer.Close(); err != nil {
			t.Error(err)
		}
	})
}
