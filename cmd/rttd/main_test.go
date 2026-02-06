package main

import (
	"bytes"
	"os"
	"path/filepath"
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
	if !strings.Contains(got, "--config path") {
		t.Fatalf("missing --config in help output: %q", got)
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

func TestLoadRuntimeConfigAutoSyncsPackets(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	if err := os.WriteFile(cfgPath, []byte(`
[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".c"]
ignore_dirs = []
`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "main.c"), []byte(`
// @rat:id=0x10, type=pose_3d
typedef struct {
  float w;
  float x;
  float y;
  float z;
} QuatSample;
`), 0o644); err != nil {
		t.Fatalf("write source: %v", err)
	}

	cfg, resolvedPath, err := loadRuntimeConfig([]string{"--config", cfgPath})
	if err != nil {
		t.Fatalf("load runtime config: %v", err)
	}
	if resolvedPath != cfgPath {
		t.Fatalf("unexpected config path: %s", resolvedPath)
	}
	if len(cfg.Packets) != 1 {
		t.Fatalf("expected 1 packet after auto-sync, got %d", len(cfg.Packets))
	}
	if cfg.Packets[0].ID != 0x10 {
		t.Fatalf("unexpected packet id: 0x%02x", cfg.Packets[0].ID)
	}
	if cfg.Packets[0].StructName != "QuatSample" {
		t.Fatalf("unexpected struct name: %s", cfg.Packets[0].StructName)
	}
}
