module github.com/wspl/demi

go 1.24.7

require (
	go.uber.org/goleak v1.3.0
	golang.org/x/sys v0.41.0
	mvdan.cc/sh/v3 v3.14.1
)

require golang.org/x/term v0.40.0 // indirect

replace mvdan.cc/sh/v3 => ./third_party/mvdan-sh
