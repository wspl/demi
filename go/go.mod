module github.com/wspl/demi/go

go 1.27

require (
	github.com/coder/websocket v1.8.15
	github.com/coreos/go-systemd/v22 v22.7.0
	github.com/klauspost/compress v1.20.1
	github.com/opencontainers/runtime-spec v1.3.0
	github.com/santhosh-tekuri/jsonschema/v6 v6.0.3
	github.com/tinylib/msgp v1.4.0
	github.com/vishvananda/netlink v1.3.1
	golang.org/x/crypto/x509roots/fallback v0.0.0-20260921070245-7a4a4d6beae2
	golang.org/x/sync v0.23.0
	golang.org/x/sys v0.48.0
	golang.org/x/tools v0.43.0
	mvdan.cc/sh/v3 v3.14.1
)

require (
	github.com/philhofer/fwd v1.2.0 // indirect
	github.com/vishvananda/netns v0.0.5 // indirect
	golang.org/x/mod v0.34.0 // indirect
	golang.org/x/term v0.45.0 // indirect
	golang.org/x/text v0.14.0 // indirect
)

replace mvdan.cc/sh/v3 => ./third_party/mvdan-sh
