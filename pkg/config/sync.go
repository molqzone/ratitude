package config

import (
	"fmt"
	"io/fs"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
)

type syncDiscoveredPacket struct {
	ID         uint16
	StructName string
	Type       string
	Packed     bool
	ByteSize   int
	Source     string
	Fields     []FieldDef
}

// SyncPackets scans C annotations and rewrites [[packets]] as generated output.
// It preserves runtime sections (for example [rttd.*]) and packet-level foxglove overrides by id.
func SyncPackets(configPath string, scanRootOverride string) (RatitudeConfig, bool, error) {
	if strings.TrimSpace(configPath) == "" {
		configPath = DefaultConfigPath
	}

	cfg, exists, err := LoadOrDefault(configPath)
	if err != nil {
		return RatitudeConfig{}, false, err
	}

	discovered, err := syncDiscoverPackets(cfg, scanRootOverride)
	if err != nil {
		return RatitudeConfig{}, false, err
	}

	merged := syncMergePackets(cfg.Packets, discovered)
	oldPackets := syncSortedPackets(cfg.Packets)
	changed := !reflect.DeepEqual(oldPackets, merged)

	cfg.Packets = merged
	if !exists || changed {
		if err := cfg.Save(configPath); err != nil {
			return RatitudeConfig{}, false, err
		}
		return cfg, true, nil
	}

	return cfg, false, nil
}

func syncMergePackets(existing []PacketDef, discovered []syncDiscoveredPacket) []PacketDef {
	oldByID := make(map[uint16]PacketDef, len(existing))
	for _, pkt := range existing {
		oldByID[pkt.ID] = pkt
	}

	merged := make([]PacketDef, 0, len(discovered))
	for _, pkt := range discovered {
		out := PacketDef{
			ID:         pkt.ID,
			StructName: pkt.StructName,
			Type:       pkt.Type,
			Packed:     pkt.Packed,
			ByteSize:   pkt.ByteSize,
			Source:     pkt.Source,
			Fields:     pkt.Fields,
			Foxglove:   map[string]any{"topic": syncDefaultTopic(pkt.StructName)},
		}
		if old, ok := oldByID[pkt.ID]; ok && old.Foxglove != nil {
			out.Foxglove = old.Foxglove
		}
		merged = append(merged, out)
	}

	sort.Slice(merged, func(i, j int) bool { return merged[i].ID < merged[j].ID })
	return merged
}

func syncSortedPackets(packets []PacketDef) []PacketDef {
	out := make([]PacketDef, len(packets))
	copy(out, packets)
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

func syncDiscoverPackets(cfg RatitudeConfig, scanRootOverride string) ([]syncDiscoveredPacket, error) {
	scanRoot := cfg.ScanRootPath()
	if strings.TrimSpace(scanRootOverride) != "" {
		scanRoot = scanRootOverride
		if !filepath.IsAbs(scanRoot) {
			scanRoot = filepath.Clean(filepath.Join(filepath.Dir(cfg.ConfigPath()), scanRoot))
		}
	}
	if scanRoot == "" {
		scanRoot = cfg.Project.ScanRoot
	}
	if !filepath.IsAbs(scanRoot) {
		scanRoot = filepath.Clean(filepath.Join(filepath.Dir(cfg.ConfigPath()), scanRoot))
	}

	exts := make(map[string]struct{}, len(cfg.Project.Extensions))
	for _, ext := range cfg.Project.Extensions {
		exts[strings.ToLower(ext)] = struct{}{}
	}
	ignores := make(map[string]struct{}, len(cfg.Project.IgnoreDirs))
	for _, name := range cfg.Project.IgnoreDirs {
		ignores[name] = struct{}{}
	}

	found := make([]syncDiscoveredPacket, 0)
	seenIDs := make(map[uint16]string)

	walkErr := filepath.WalkDir(scanRoot, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}

		if d.IsDir() {
			if path != scanRoot {
				if !cfg.Project.Recursive {
					return filepath.SkipDir
				}
				if _, skip := ignores[d.Name()]; skip {
					return filepath.SkipDir
				}
			}
			return nil
		}

		if _, ok := exts[strings.ToLower(filepath.Ext(path))]; !ok {
			return nil
		}

		packets, err := syncParseTaggedFile(path, scanRoot)
		if err != nil {
			return err
		}
		for _, pkt := range packets {
			if prev, dup := seenIDs[pkt.ID]; dup {
				return fmt.Errorf("duplicate packet id 0x%02x in %s and %s", pkt.ID, prev, pkt.Source)
			}
			seenIDs[pkt.ID] = pkt.Source
			found = append(found, pkt)
		}
		return nil
	})
	if walkErr != nil {
		return nil, walkErr
	}

	sort.Slice(found, func(i, j int) bool { return found[i].ID < found[j].ID })
	return found, nil
}

func syncCTypeSize(raw string) (int, bool) {
	switch syncNormalizeCType(raw) {
	case "float":
		return 4, true
	case "double":
		return 8, true
	case "int8_t", "uint8_t", "bool", "_bool":
		return 1, true
	case "int16_t", "uint16_t":
		return 2, true
	case "int32_t", "uint32_t":
		return 4, true
	case "int64_t", "uint64_t":
		return 8, true
	default:
		return 0, false
	}
}

func syncNormalizeCType(raw string) string {
	s := strings.ToLower(strings.TrimSpace(raw))
	s = strings.ReplaceAll(s, "\t", " ")
	for strings.Contains(s, "  ") {
		s = strings.ReplaceAll(s, "  ", " ")
	}
	s = strings.TrimPrefix(s, "const ")
	s = strings.TrimPrefix(s, "volatile ")
	return strings.TrimSpace(s)
}

func syncAlignUp(value int, align int) int {
	if align <= 1 {
		return value
	}
	rem := value % align
	if rem == 0 {
		return value
	}
	return value + (align - rem)
}

func syncDefaultTopic(structName string) string {
	return "/rat/" + strings.ToLower(structName)
}
