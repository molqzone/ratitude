package main

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"ratitude/pkg/config"
)

func TestSyncScansRecursivelyAndIgnoresDirs(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWrite(t, cfgPath, `[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".h", ".c"]
ignore_dirs = ["Drivers", ".git", "build"]
`)

	srcPath := filepath.Join(dir, "Core", "Src", "main.c")
	mustWrite(t, srcPath, `
// @rat:id=0x01, type=plot
typedef struct {
  int32_t value;
  uint32_t tick_ms;
} RatSample;
`)

	ignoredPath := filepath.Join(dir, "Drivers", "ignored.c")
	mustWrite(t, ignoredPath, `
// @rat:id=0x02, type=plot
typedef struct {
  int32_t ignored;
} Ignored;
`)

	var out bytes.Buffer
	var errOut bytes.Buffer
	code := run([]string{"sync", "--config", cfgPath}, &out, &errOut)
	if code != 0 {
		t.Fatalf("sync failed code=%d stderr=%s", code, errOut.String())
	}

	cfg, err := config.Load(cfgPath)
	if err != nil {
		t.Fatalf("load synced config: %v", err)
	}

	if len(cfg.Packets) != 1 {
		t.Fatalf("expected 1 packet, got %d", len(cfg.Packets))
	}
	pkt := cfg.Packets[0]
	if pkt.ID != 0x01 {
		t.Fatalf("unexpected packet id: 0x%02x", pkt.ID)
	}
	if pkt.StructName != "RatSample" {
		t.Fatalf("unexpected struct name: %s", pkt.StructName)
	}
	if pkt.ByteSize != 8 {
		t.Fatalf("unexpected byte size: %d", pkt.ByteSize)
	}
	if len(pkt.Fields) != 2 || pkt.Fields[1].Offset != 4 {
		t.Fatalf("unexpected fields: %#v", pkt.Fields)
	}
}

func TestSyncComputesStructLayoutPackedAndUnpacked(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWrite(t, cfgPath, `[project]
name = "layout"
scan_root = "."
recursive = true
extensions = [".h"]
ignore_dirs = []
`)

	mustWrite(t, filepath.Join(dir, "layout.h"), `
// @rat:id=0x10, type=json
typedef struct {
  uint8_t a;
  uint32_t b;
} NaturalLayout;

// @rat:id=0x11, type=json
typedef struct {
  uint8_t a;
  uint32_t b;
} __attribute__((packed)) PackedLayout;
`)

	var out bytes.Buffer
	var errOut bytes.Buffer
	code := run([]string{"sync", "--config", cfgPath}, &out, &errOut)
	if code != 0 {
		t.Fatalf("sync failed code=%d stderr=%s", code, errOut.String())
	}

	cfg, err := config.Load(cfgPath)
	if err != nil {
		t.Fatalf("load synced config: %v", err)
	}
	if len(cfg.Packets) != 2 {
		t.Fatalf("expected 2 packets, got %d", len(cfg.Packets))
	}

	natural := cfg.Packets[0]
	if natural.ID != 0x10 || natural.ByteSize != 8 || natural.Packed {
		t.Fatalf("unexpected natural layout: %#v", natural)
	}
	if natural.Fields[1].Offset != 4 {
		t.Fatalf("natural layout offset mismatch: %#v", natural.Fields)
	}

	packed := cfg.Packets[1]
	if packed.ID != 0x11 || packed.ByteSize != 5 || !packed.Packed {
		t.Fatalf("unexpected packed layout: %#v", packed)
	}
	if packed.Fields[1].Offset != 1 {
		t.Fatalf("packed layout offset mismatch: %#v", packed.Fields)
	}
}

func TestSyncRemovesStalePackets(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWrite(t, cfgPath, `[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".h"]
ignore_dirs = []

[[packets]]
id = 0x99
struct_name = "OldPacket"
type = "json"
packed = true
byte_size = 1

[[packets.fields]]
name = "x"
c_type = "uint8_t"
offset = 0
size = 1
`)

	mustWrite(t, filepath.Join(dir, "main.h"), `
// @rat:id=0x01, type=plot
typedef struct {
  int32_t value;
} NewPacket;
`)

	var out bytes.Buffer
	var errOut bytes.Buffer
	code := run([]string{"sync", "--config", cfgPath}, &out, &errOut)
	if code != 0 {
		t.Fatalf("sync failed code=%d stderr=%s", code, errOut.String())
	}

	cfg, err := config.Load(cfgPath)
	if err != nil {
		t.Fatalf("load synced config: %v", err)
	}
	if len(cfg.Packets) != 1 || cfg.Packets[0].ID != 0x01 {
		t.Fatalf("stale packets were not removed: %#v", cfg.Packets)
	}
}

func TestSyncFailsOnDuplicatePacketID(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWrite(t, cfgPath, `[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".h", ".c"]
ignore_dirs = [".git"]
`)

	mustWrite(t, filepath.Join(dir, "a.c"), `
// @rat:id=0x10, type=plot
typedef struct { int32_t v; } A;
`)
	mustWrite(t, filepath.Join(dir, "b.c"), `
// @rat:id=0x10, type=json
typedef struct { uint32_t v; } B;
`)

	var out bytes.Buffer
	var errOut bytes.Buffer
	code := run([]string{"sync", "--config", cfgPath}, &out, &errOut)
	if code == 0 {
		t.Fatalf("expected duplicate id failure")
	}
}

func TestSyncSupportsBlockCommentTag(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWrite(t, cfgPath, `[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".h"]
ignore_dirs = []
`)

	mustWrite(t, filepath.Join(dir, "packet.h"), `
/*
 * telemetry definition
 * @rat:id=0x21, type=json
 */
typedef struct {
  uint16_t voltage_mv;
  uint16_t current_ma;
} BatteryReading;
`)

	var out bytes.Buffer
	var errOut bytes.Buffer
	code := run([]string{"sync", "--config", cfgPath}, &out, &errOut)
	if code != 0 {
		t.Fatalf("sync failed code=%d stderr=%s", code, errOut.String())
	}

	cfg, err := config.Load(cfgPath)
	if err != nil {
		t.Fatalf("load synced config: %v", err)
	}
	if len(cfg.Packets) != 1 || cfg.Packets[0].ID != 0x21 {
		t.Fatalf("expected block comment tag packet 0x21, got %#v", cfg.Packets)
	}
}

func TestSyncFailsWhenMultipleTagsInSingleCommentBlock(t *testing.T) {
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "ratitude.toml")
	mustWrite(t, cfgPath, `[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".c"]
ignore_dirs = []
`)

	mustWrite(t, filepath.Join(dir, "bad.c"), `
/* @rat:id=0x30, type=plot @rat:id=0x31, type=json */
typedef struct {
  uint32_t value;
} BadPacket;
`)

	var out bytes.Buffer
	var errOut bytes.Buffer
	code := run([]string{"sync", "--config", cfgPath}, &out, &errOut)
	if code == 0 {
		t.Fatalf("expected failure for multiple tags in one comment block")
	}
	if got := errOut.String(); got == "" {
		t.Fatalf("expected stderr explaining failure")
	}
}
func mustWrite(t *testing.T, path string, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", path, err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}
