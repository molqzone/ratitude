package config

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	toml "github.com/pelletier/go-toml/v2"
)

const DefaultConfigPath = "firmware/example/stm32f4_rtt/ratitude.toml"

type RatitudeConfig struct {
	Project    ProjectConfig `toml:"project"`
	RTTD       RTTDConfig    `toml:"rttd"`
	Packets    []PacketDef   `toml:"packets"`
	configPath string        `toml:"-"`
	scanRoot   string        `toml:"-"`
}

type ProjectConfig struct {
	Name       string   `toml:"name"`
	SourceDir  string   `toml:"source_dir,omitempty"`
	ScanRoot   string   `toml:"scan_root"`
	Recursive  bool     `toml:"recursive"`
	Extensions []string `toml:"extensions"`
	IgnoreDirs []string `toml:"ignore_dirs"`
}

type RTTDConfig struct {
	TextID   uint16         `toml:"text_id"`
	Server   ServerConfig   `toml:"server"`
	Foxglove FoxgloveConfig `toml:"foxglove"`
}

type ServerConfig struct {
	Addr      string `toml:"addr"`
	Reconnect string `toml:"reconnect"`
	Buf       int    `toml:"buf"`
	ReaderBuf int    `toml:"reader_buf"`
}

type FoxgloveConfig struct {
	WSAddr      string `toml:"ws_addr"`
	Topic       string `toml:"topic"`
	SchemaName  string `toml:"schema_name"`
	QuatID      uint16 `toml:"quat_id"`
	TempID      uint16 `toml:"temp_id"`
	MarkerTopic string `toml:"marker_topic"`
	ParentFrame string `toml:"parent_frame"`
	FrameID     string `toml:"frame_id"`
	ImagePath   string `toml:"image_path,omitempty"`
	ImageFrame  string `toml:"image_frame,omitempty"`
	ImageFormat string `toml:"image_format,omitempty"`
	LogTopic    string `toml:"log_topic"`
	LogName     string `toml:"log_name"`
}

type PacketDef struct {
	ID         uint16         `toml:"id"`
	StructName string         `toml:"struct_name"`
	Type       string         `toml:"type"`
	Packed     bool           `toml:"packed"`
	ByteSize   int            `toml:"byte_size"`
	Source     string         `toml:"source,omitempty"`
	Fields     []FieldDef     `toml:"fields"`
	Foxglove   map[string]any `toml:"foxglove,omitempty"`
}

type FieldDef struct {
	Name   string `toml:"name"`
	CType  string `toml:"c_type"`
	Offset int    `toml:"offset"`
	Size   int    `toml:"size"`
}

func Default() RatitudeConfig {
	return RatitudeConfig{
		Project: ProjectConfig{
			Name:       "stm32f4_rtt",
			ScanRoot:   ".",
			Recursive:  true,
			Extensions: []string{".h", ".c"},
			IgnoreDirs: []string{"Drivers", ".git", "build"},
		},
		RTTD: RTTDConfig{
			TextID: 0xFF,
			Server: ServerConfig{
				Addr:      "127.0.0.1:19021",
				Reconnect: "1s",
				Buf:       256,
				ReaderBuf: 64 * 1024,
			},
			Foxglove: FoxgloveConfig{
				WSAddr:      "127.0.0.1:8765",
				Topic:       "ratitude/packet",
				SchemaName:  "ratitude.Packet",
				QuatID:      0x10,
				TempID:      0x20,
				MarkerTopic: "/visualization_marker",
				ParentFrame: "world",
				FrameID:     "base_link",
				ImagePath:   "D:/Repos/ratitude/demo.jpg",
				ImageFrame:  "camera",
				ImageFormat: "jpeg",
				LogTopic:    "/ratitude/log",
				LogName:     "ratitude",
			},
		},
		Packets: []PacketDef{},
	}
}

func Load(path string) (RatitudeConfig, error) {
	cfg, exists, err := LoadOrDefault(path)
	if err != nil {
		return RatitudeConfig{}, err
	}
	if !exists {
		return RatitudeConfig{}, os.ErrNotExist
	}
	return cfg, nil
}

func LoadOrDefault(path string) (RatitudeConfig, bool, error) {
	cfg := Default()
	cfg.configPath = path

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			cfg.normalize(path)
			return cfg, false, nil
		}
		return RatitudeConfig{}, false, fmt.Errorf("read config: %w", err)
	}

	if err := toml.Unmarshal(data, &cfg); err != nil {
		return RatitudeConfig{}, true, fmt.Errorf("parse config: %w", err)
	}
	cfg.configPath = path
	cfg.normalize(path)

	if err := cfg.Validate(); err != nil {
		return RatitudeConfig{}, true, err
	}
	return cfg, true, nil
}

func (cfg *RatitudeConfig) Save(path string) error {
	cfg.normalize(path)
	if err := cfg.Validate(); err != nil {
		return err
	}

	sort.Slice(cfg.Packets, func(i, j int) bool {
		return cfg.Packets[i].ID < cfg.Packets[j].ID
	})

	data, err := toml.Marshal(cfg)
	if err != nil {
		return fmt.Errorf("marshal config: %w", err)
	}

	dir := filepath.Dir(path)
	if dir != "" && dir != "." {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return fmt.Errorf("create config directory: %w", err)
		}
	}

	if err := os.WriteFile(path, data, 0o644); err != nil {
		return fmt.Errorf("write config: %w", err)
	}
	return nil
}

func (cfg *RatitudeConfig) ConfigPath() string {
	return cfg.configPath
}

func (cfg *RatitudeConfig) ScanRootPath() string {
	return cfg.scanRoot
}

func (cfg *RatitudeConfig) Validate() error {
	if cfg.RTTD.TextID > 0xFF {
		return fmt.Errorf("rttd.text_id out of range: 0x%x", cfg.RTTD.TextID)
	}
	if cfg.RTTD.Foxglove.QuatID > 0xFF {
		return fmt.Errorf("rttd.foxglove.quat_id out of range: 0x%x", cfg.RTTD.Foxglove.QuatID)
	}
	if cfg.RTTD.Foxglove.TempID > 0xFF {
		return fmt.Errorf("rttd.foxglove.temp_id out of range: 0x%x", cfg.RTTD.Foxglove.TempID)
	}

	seen := make(map[uint16]struct{}, len(cfg.Packets))
	for _, pkt := range cfg.Packets {
		if pkt.ID > 0xFF {
			return fmt.Errorf("packet id out of range: 0x%x", pkt.ID)
		}
		if _, ok := seen[pkt.ID]; ok {
			return fmt.Errorf("duplicate packet id: 0x%02x", pkt.ID)
		}
		seen[pkt.ID] = struct{}{}
		if pkt.StructName == "" {
			return fmt.Errorf("packet 0x%02x has empty struct_name", pkt.ID)
		}
		if pkt.ByteSize < 0 {
			return fmt.Errorf("packet 0x%02x has invalid byte_size", pkt.ID)
		}
		for _, field := range pkt.Fields {
			if field.Name == "" {
				return fmt.Errorf("packet 0x%02x has field with empty name", pkt.ID)
			}
			if field.Size <= 0 {
				return fmt.Errorf("packet 0x%02x field %s has invalid size", pkt.ID, field.Name)
			}
			if field.Offset < 0 {
				return fmt.Errorf("packet 0x%02x field %s has invalid offset", pkt.ID, field.Name)
			}
		}
	}
	return nil
}

func (cfg *RatitudeConfig) normalize(path string) {
	def := Default()

	if cfg.Project.Name == "" {
		cfg.Project.Name = def.Project.Name
	}
	if cfg.Project.ScanRoot == "" {
		if cfg.Project.SourceDir != "" {
			cfg.Project.ScanRoot = cfg.Project.SourceDir
		} else {
			cfg.Project.ScanRoot = def.Project.ScanRoot
		}
	}
	cfg.Project.SourceDir = ""
	if len(cfg.Project.Extensions) == 0 {
		cfg.Project.Extensions = append([]string(nil), def.Project.Extensions...)
	}
	if len(cfg.Project.IgnoreDirs) == 0 {
		cfg.Project.IgnoreDirs = append([]string(nil), def.Project.IgnoreDirs...)
	}

	if cfg.RTTD.Server.Addr == "" {
		cfg.RTTD.Server.Addr = def.RTTD.Server.Addr
	}
	if cfg.RTTD.Server.Reconnect == "" {
		cfg.RTTD.Server.Reconnect = def.RTTD.Server.Reconnect
	}
	if cfg.RTTD.Server.Buf <= 0 {
		cfg.RTTD.Server.Buf = def.RTTD.Server.Buf
	}
	if cfg.RTTD.Server.ReaderBuf <= 0 {
		cfg.RTTD.Server.ReaderBuf = def.RTTD.Server.ReaderBuf
	}

	if cfg.RTTD.Foxglove.WSAddr == "" {
		cfg.RTTD.Foxglove.WSAddr = def.RTTD.Foxglove.WSAddr
	}
	if cfg.RTTD.Foxglove.Topic == "" {
		cfg.RTTD.Foxglove.Topic = def.RTTD.Foxglove.Topic
	}
	if cfg.RTTD.Foxglove.SchemaName == "" {
		cfg.RTTD.Foxglove.SchemaName = def.RTTD.Foxglove.SchemaName
	}
	if cfg.RTTD.Foxglove.MarkerTopic == "" {
		cfg.RTTD.Foxglove.MarkerTopic = def.RTTD.Foxglove.MarkerTopic
	}
	if cfg.RTTD.Foxglove.ParentFrame == "" {
		cfg.RTTD.Foxglove.ParentFrame = def.RTTD.Foxglove.ParentFrame
	}
	if cfg.RTTD.Foxglove.FrameID == "" {
		cfg.RTTD.Foxglove.FrameID = def.RTTD.Foxglove.FrameID
	}
	if cfg.RTTD.Foxglove.ImagePath == "" {
		cfg.RTTD.Foxglove.ImagePath = def.RTTD.Foxglove.ImagePath
	}
	if cfg.RTTD.Foxglove.ImageFrame == "" {
		cfg.RTTD.Foxglove.ImageFrame = def.RTTD.Foxglove.ImageFrame
	}
	if cfg.RTTD.Foxglove.ImageFormat == "" {
		cfg.RTTD.Foxglove.ImageFormat = def.RTTD.Foxglove.ImageFormat
	}
	if cfg.RTTD.Foxglove.LogTopic == "" {
		cfg.RTTD.Foxglove.LogTopic = def.RTTD.Foxglove.LogTopic
	}
	if cfg.RTTD.Foxglove.LogName == "" {
		cfg.RTTD.Foxglove.LogName = def.RTTD.Foxglove.LogName
	}

	if path == "" {
		path = cfg.configPath
	}
	if path == "" {
		path = DefaultConfigPath
	}

	cfg.configPath = path
	baseDir := filepath.Dir(path)
	if baseDir == "" {
		baseDir = "."
	}

	scanRoot := cfg.Project.ScanRoot
	if !filepath.IsAbs(scanRoot) {
		scanRoot = filepath.Join(baseDir, scanRoot)
	}
	scanRoot = filepath.Clean(scanRoot)
	if abs, err := filepath.Abs(scanRoot); err == nil {
		scanRoot = abs
	}
	cfg.scanRoot = scanRoot

	for i := range cfg.Project.Extensions {
		ext := strings.TrimSpace(cfg.Project.Extensions[i])
		if ext == "" {
			continue
		}
		if !strings.HasPrefix(ext, ".") {
			ext = "." + ext
		}
		cfg.Project.Extensions[i] = strings.ToLower(ext)
	}
}
