package config_test

import (
	"os"
	"path/filepath"
	"testing"

	"ratitude/pkg/config"
)

func TestLoadOrDefaultResolvesScanRootRelativeToConfig(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "example", "ratitude.toml")
	mustMkdirAll(t, filepath.Dir(cfgPath))

	content := `
[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".h", ".c"]
ignore_dirs = ["Drivers", ".git", "build"]
`
	mustWriteFile(t, cfgPath, content)

	cfg, _, err := config.LoadOrDefault(cfgPath)
	if err != nil {
		t.Fatalf("load config: %v", err)
	}

	want := filepath.Clean(filepath.Dir(cfgPath))
	if got := cfg.ScanRootPath(); got != want {
		t.Fatalf("unexpected scan root: got %q want %q", got, want)
	}
}

func TestLoadOrDefaultFillsDefaults(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWriteFile(t, cfgPath, "[project]\nname='demo'\nscan_root='.'\n")

	cfg, _, err := config.LoadOrDefault(cfgPath)
	if err != nil {
		t.Fatalf("load config: %v", err)
	}

	if cfg.RTTD.Server.Addr == "" {
		t.Fatalf("expected default server addr")
	}
	if cfg.RTTD.Foxglove.WSAddr == "" {
		t.Fatalf("expected default foxglove ws addr")
	}
	if len(cfg.Project.Extensions) == 0 {
		t.Fatalf("expected default extensions")
	}
}

func TestSyncPacketsPreservesRuntimeSectionsAndFoxgloveOverride(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWriteFile(t, cfgPath, `
[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".c"]
ignore_dirs = []

[rttd.server]
addr = "10.0.0.1:1234"

[[packets]]
id = 0x01
struct_name = "OldName"
type = "plot"
packed = true
byte_size = 1
source = "old.c"

[[packets.fields]]
name = "x"
c_type = "uint8_t"
offset = 0
size = 1

[packets.foxglove]
topic = "/custom/topic"

[[packets]]
id = 0x99
struct_name = "Stale"
type = "json"
packed = true
byte_size = 1

[[packets.fields]]
name = "y"
c_type = "uint8_t"
offset = 0
size = 1
`)

	src := filepath.Join(dir, "main.c")
	mustWriteFile(t, src, `
// @rat:id=0x01, type=plot
typedef struct {
  int32_t value;
  uint32_t tick_ms;
} RatSample;
`)

	cfg, changed, err := config.SyncPackets(cfgPath, "")
	if err != nil {
		t.Fatalf("sync packets: %v", err)
	}
	if !changed {
		t.Fatalf("expected changed=true on first sync")
	}
	if got := cfg.RTTD.Server.Addr; got != "10.0.0.1:1234" {
		t.Fatalf("server addr should be preserved, got %q", got)
	}
	if len(cfg.Packets) != 1 {
		t.Fatalf("expected stale packets removed, got %d", len(cfg.Packets))
	}
	if got := cfg.Packets[0].StructName; got != "RatSample" {
		t.Fatalf("unexpected struct name: %s", got)
	}
	if got := cfg.Packets[0].Foxglove["topic"]; got != "/custom/topic" {
		t.Fatalf("foxglove override should be preserved, got %#v", got)
	}

	_, changed, err = config.SyncPackets(cfgPath, "")
	if err != nil {
		t.Fatalf("second sync packets: %v", err)
	}
	if changed {
		t.Fatalf("expected changed=false on second sync")
	}
}

func mustMkdirAll(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(path, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
}

func mustWriteFile(t *testing.T, path string, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write file: %v", err)
	}
}
