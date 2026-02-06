//go:build !cgo

package config

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

var (
	syncTagBodyRegexp    = regexp.MustCompile(`@rat:id=(0x[0-9A-Fa-f]+)\s*,\s*type=([A-Za-z_][A-Za-z0-9_]*)`)
	syncCommentRegexp    = regexp.MustCompile(`(?m)//[^\r\n]*|(?s:/\*.*?\*/)`)
	syncStructRegexp     = regexp.MustCompile(`(?s)typedef\s+struct\s*\{(.*?)\}\s*((?:__attribute__\s*\(\(\s*packed\s*\)\)\s*)?)([A-Za-z_][A-Za-z0-9_]*)\s*;`)
	syncIdentRegexp      = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)
	syncBlockCommentsRe  = regexp.MustCompile(`(?s)/\*.*?\*/`)
	syncLineCommentsRe   = regexp.MustCompile(`(?m)//.*$`)
	syncPackedWordRegexp = regexp.MustCompile(`\bpacked\b`)
)

type syncStructMatch struct {
	start      int
	body       string
	packedAttr string
	name       string
}

type syncParsedField struct {
	Name  string
	CType string
	Size  int
}

type syncTagMatch struct {
	endByte int
	id      uint16
	pktType string
}

func syncParseTaggedFile(path string, scanRoot string) ([]syncDiscoveredPacket, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	content := string(data)

	tags, err := syncExtractTagsFromComments(content, path)
	if err != nil {
		return nil, err
	}
	if len(tags) == 0 {
		return nil, nil
	}

	structMatchesRaw := syncStructRegexp.FindAllStringSubmatchIndex(content, -1)
	if len(structMatchesRaw) == 0 {
		return nil, fmt.Errorf("found @rat tags in %s but no typedef struct definitions", path)
	}

	structs := make([]syncStructMatch, 0, len(structMatchesRaw))
	for _, m := range structMatchesRaw {
		structs = append(structs, syncStructMatch{
			start:      m[0],
			body:       content[m[2]:m[3]],
			packedAttr: content[m[4]:m[5]],
			name:       content[m[6]:m[7]],
		})
	}

	usedStructs := make(map[int]struct{})
	out := make([]syncDiscoveredPacket, 0, len(tags))

	for _, tag := range tags {
		structIdx := -1
		for i, st := range structs {
			if st.start >= tag.endByte {
				if _, used := usedStructs[i]; used {
					continue
				}
				structIdx = i
				break
			}
		}
		if structIdx < 0 {
			return nil, fmt.Errorf("@rat tag id=0x%02x in %s has no following typedef struct", tag.id, path)
		}
		usedStructs[structIdx] = struct{}{}

		st := structs[structIdx]
		packed := syncPackedWordRegexp.MatchString(strings.ToLower(st.packedAttr))
		fields, byteSize, err := syncParseStructFieldsFallback(st.body, packed, path, st.name)
		if err != nil {
			return nil, err
		}

		source, relErr := filepath.Rel(scanRoot, path)
		if relErr != nil {
			source = path
		}
		source = filepath.ToSlash(source)

		out = append(out, syncDiscoveredPacket{
			ID:         tag.id,
			StructName: st.name,
			Type:       tag.pktType,
			Packed:     packed,
			ByteSize:   byteSize,
			Source:     source,
			Fields:     fields,
		})
	}

	return out, nil
}

func syncExtractTagsFromComments(content string, path string) ([]syncTagMatch, error) {
	commentMatches := syncCommentRegexp.FindAllStringIndex(content, -1)
	tags := make([]syncTagMatch, 0)
	for _, cm := range commentMatches {
		comment := content[cm[0]:cm[1]]
		matches := syncTagBodyRegexp.FindAllStringSubmatchIndex(comment, -1)
		if len(matches) == 0 {
			continue
		}
		if len(matches) > 1 {
			return nil, fmt.Errorf("multiple @rat tags in one comment block in %s", path)
		}
		m := matches[0]
		idStr := comment[m[2]:m[3]]
		pktType := comment[m[4]:m[5]]
		id64, err := strconv.ParseUint(idStr, 0, 16)
		if err != nil {
			return nil, fmt.Errorf("invalid packet id %q in %s", idStr, path)
		}
		if id64 > 0xFF {
			return nil, fmt.Errorf("packet id out of range (%s) in %s", idStr, path)
		}
		tags = append(tags, syncTagMatch{endByte: cm[0] + m[1], id: uint16(id64), pktType: pktType})
	}

	sort.Slice(tags, func(i, j int) bool {
		if tags[i].endByte == tags[j].endByte {
			return tags[i].id < tags[j].id
		}
		return tags[i].endByte < tags[j].endByte
	})
	return tags, nil
}

func syncParseStructFieldsFallback(body string, packed bool, path string, structName string) ([]FieldDef, int, error) {
	clean := syncStripCommentsFallback(body)
	segments := strings.Split(clean, ";")

	parsed := make([]syncParsedField, 0)
	for _, seg := range segments {
		line := strings.TrimSpace(seg)
		if line == "" {
			continue
		}
		if strings.ContainsAny(line, "*[]:") {
			return nil, 0, fmt.Errorf("unsupported field syntax in %s (%s): %q", path, structName, line)
		}
		if strings.Contains(line, "union") || strings.Contains(line, "struct") {
			return nil, 0, fmt.Errorf("unsupported nested declaration in %s (%s): %q", path, structName, line)
		}

		tokens := strings.Fields(line)
		if len(tokens) < 2 {
			return nil, 0, fmt.Errorf("invalid field declaration in %s (%s): %q", path, structName, line)
		}
		name := tokens[len(tokens)-1]
		ctype := strings.Join(tokens[:len(tokens)-1], " ")
		if !syncIdentRegexp.MatchString(name) {
			return nil, 0, fmt.Errorf("invalid field name in %s (%s): %q", path, structName, name)
		}

		size, ok := syncCTypeSize(ctype)
		if !ok {
			return nil, 0, fmt.Errorf("unsupported c type in %s (%s): %q", path, structName, ctype)
		}

		parsed = append(parsed, syncParsedField{Name: name, CType: syncNormalizeCType(ctype), Size: size})
	}

	if len(parsed) == 0 {
		return nil, 0, fmt.Errorf("struct %s in %s has no supported fields", structName, path)
	}

	fields := make([]FieldDef, 0, len(parsed))
	offset := 0
	maxAlign := 1
	for _, f := range parsed {
		align := 1
		if !packed {
			align = f.Size
			if align > maxAlign {
				maxAlign = align
			}
			offset = syncAlignUp(offset, align)
		}

		fields = append(fields, FieldDef{Name: f.Name, CType: f.CType, Offset: offset, Size: f.Size})
		offset += f.Size
	}

	total := offset
	if !packed {
		total = syncAlignUp(total, maxAlign)
	}
	return fields, total, nil
}

func syncStripCommentsFallback(in string) string {
	out := syncBlockCommentsRe.ReplaceAllString(in, "")
	out = syncLineCommentsRe.ReplaceAllString(out, "")
	return out
}
