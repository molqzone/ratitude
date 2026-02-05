package main

import (
    "bytes"
    "strings"
    "testing"
)

func TestRun(t *testing.T) {
    var buf bytes.Buffer
    run(&buf)

    got := strings.TrimSpace(buf.String())
    want := "hello world"
    if got != want {
        t.Fatalf("unexpected output: got %q want %q", got, want)
    }
}