package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestRunHelp(t *testing.T) {
	var out bytes.Buffer
	var err bytes.Buffer

	code := run([]string{"--help"}, &out, &err)
	if code != 0 {
		t.Fatalf("expected exit code 0, got %d", code)
	}

	got := strings.TrimSpace(out.String())
	if !strings.Contains(got, "rttd server") {
		t.Fatalf("unexpected help output: %q", got)
	}
	if !strings.Contains(got, "rttd foxglove") {
		t.Fatalf("unexpected help output: %q", got)
	}
	if !strings.Contains(got, "--log-topic /ratitude/log") {
		t.Fatalf("missing --log-topic in help output: %q", got)
	}
	if !strings.Contains(got, "--log-name ratitude") {
		t.Fatalf("missing --log-name in help output: %q", got)
	}
	if !strings.Contains(got, "--temp-id 0x20") {
		t.Fatalf("missing --temp-id in help output: %q", got)
	}
}
