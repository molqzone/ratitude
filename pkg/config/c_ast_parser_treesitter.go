//go:build cgo

package config

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"

	sitter "github.com/smacker/go-tree-sitter"
	tsc "github.com/smacker/go-tree-sitter/c"
)

var (
	syncTagBodyRegexp    = regexp.MustCompile(`@rat:id=(0x[0-9A-Fa-f]+)\s*,\s*type=([A-Za-z_][A-Za-z0-9_]*)`)
	syncPackedWordRegexp = regexp.MustCompile(`\bpacked\b`)
	syncFieldNameRegexp  = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)
)

type syncTagMatch struct {
	endByte uint32
	id      uint16
	pktType string
}

type syncStructDef struct {
	startByte uint32
	name      string
	packed    bool
	byteSize  int
	fields    []FieldDef
}

type syncStructField struct {
	name  string
	ctype string
	size  int
}

func syncParseTaggedFile(path string, scanRoot string) ([]syncDiscoveredPacket, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}

	root := sitter.Parse(data, tsc.GetLanguage())
	tags, err := syncCollectCommentTags(root, data, path)
	if err != nil {
		return nil, err
	}
	if len(tags) == 0 {
		return nil, nil
	}

	structs, err := syncCollectTypeDefinitions(root, data, path)
	if err != nil {
		return nil, err
	}
	if len(structs) == 0 {
		return nil, fmt.Errorf("found @rat tags in %s but no typedef struct definitions", path)
	}

	usedStructs := make(map[int]struct{})
	out := make([]syncDiscoveredPacket, 0, len(tags))

	for _, tag := range tags {
		structIdx := -1
		for i, st := range structs {
			if st.startByte >= tag.endByte {
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

		source, relErr := filepath.Rel(scanRoot, path)
		if relErr != nil {
			source = path
		}
		source = filepath.ToSlash(source)

		out = append(out, syncDiscoveredPacket{
			ID:         tag.id,
			StructName: st.name,
			Type:       tag.pktType,
			Packed:     st.packed,
			ByteSize:   st.byteSize,
			Source:     source,
			Fields:     st.fields,
		})
	}

	return out, nil
}

func syncCollectCommentTags(root *sitter.Node, data []byte, path string) ([]syncTagMatch, error) {
	tags := make([]syncTagMatch, 0)
	err := syncWalkNode(root, func(node *sitter.Node) error {
		if node.Type() != "comment" {
			return nil
		}
		comment := node.Content(data)
		matches := syncTagBodyRegexp.FindAllStringSubmatchIndex(comment, -1)
		if len(matches) == 0 {
			return nil
		}
		if len(matches) > 1 {
			line := node.StartPoint().Row + 1
			return fmt.Errorf("multiple @rat tags in one comment block at %s:%d", path, line)
		}

		match := matches[0]
		idStr := comment[match[2]:match[3]]
		pktType := comment[match[4]:match[5]]

		id64, err := strconv.ParseUint(idStr, 0, 16)
		if err != nil {
			line := node.StartPoint().Row + 1
			return fmt.Errorf("invalid packet id %q in %s:%d", idStr, path, line)
		}
		if id64 > 0xFF {
			line := node.StartPoint().Row + 1
			return fmt.Errorf("packet id out of range (%s) in %s:%d", idStr, path, line)
		}

		tagEnd := node.StartByte() + uint32(match[1])
		tags = append(tags, syncTagMatch{endByte: tagEnd, id: uint16(id64), pktType: pktType})
		return nil
	})
	if err != nil {
		return nil, err
	}

	sort.Slice(tags, func(i, j int) bool {
		if tags[i].endByte == tags[j].endByte {
			return tags[i].id < tags[j].id
		}
		return tags[i].endByte < tags[j].endByte
	})
	return tags, nil
}

func syncCollectTypeDefinitions(root *sitter.Node, data []byte, path string) ([]syncStructDef, error) {
	structs := make([]syncStructDef, 0)
	err := syncWalkNode(root, func(node *sitter.Node) error {
		if node.Type() != "type_definition" {
			return nil
		}
		st, ok, err := syncParseTypeDefinitionNode(node, data, path)
		if err != nil {
			return err
		}
		if ok {
			structs = append(structs, st)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}

	sort.Slice(structs, func(i, j int) bool { return structs[i].startByte < structs[j].startByte })
	return structs, nil
}

func syncParseTypeDefinitionNode(node *sitter.Node, data []byte, path string) (syncStructDef, bool, error) {
	typeNode := node.ChildByFieldName("type")
	if typeNode == nil || typeNode.IsNull() {
		return syncStructDef{}, false, nil
	}

	structNode := syncFindFirstNodeByType(typeNode, "struct_specifier")
	if structNode == nil || structNode.IsNull() {
		return syncStructDef{}, false, nil
	}

	bodyNode := structNode.ChildByFieldName("body")
	if bodyNode == nil || bodyNode.IsNull() {
		return syncStructDef{}, false, nil
	}

	declNodes := syncChildNodesByFieldName(node, "declarator")
	if len(declNodes) != 1 {
		line := node.StartPoint().Row + 1
		return syncStructDef{}, false, fmt.Errorf("typedef struct in %s:%d must have exactly one declarator", path, line)
	}

	structName, err := syncExtractDeclaratorName(declNodes[0], data)
	if err != nil {
		line := node.StartPoint().Row + 1
		return syncStructDef{}, false, fmt.Errorf("invalid typedef struct declarator in %s:%d: %w", path, line, err)
	}

	packed := syncPackedWordRegexp.MatchString(strings.ToLower(node.Content(data)))
	fields, byteSize, err := syncParseStructFieldsFromAST(bodyNode, data, packed, path, structName)
	if err != nil {
		return syncStructDef{}, false, err
	}

	return syncStructDef{
		startByte: node.StartByte(),
		name:      structName,
		packed:    packed,
		byteSize:  byteSize,
		fields:    fields,
	}, true, nil
}

func syncParseStructFieldsFromAST(body *sitter.Node, data []byte, packed bool, path string, structName string) ([]FieldDef, int, error) {
	parsed := make([]syncStructField, 0)
	for i := 0; i < int(body.NamedChildCount()); i++ {
		child := body.NamedChild(i)
		if child == nil || child.IsNull() {
			continue
		}
		if child.Type() != "field_declaration" {
			continue
		}

		field, err := syncParseFieldDeclarationNode(child, data, path, structName)
		if err != nil {
			return nil, 0, err
		}
		parsed = append(parsed, field)
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
			align = f.size
			if align > maxAlign {
				maxAlign = align
			}
			offset = syncAlignUp(offset, align)
		}
		fields = append(fields, FieldDef{Name: f.name, CType: f.ctype, Offset: offset, Size: f.size})
		offset += f.size
	}

	total := offset
	if !packed {
		total = syncAlignUp(total, maxAlign)
	}
	return fields, total, nil
}

func syncParseFieldDeclarationNode(node *sitter.Node, data []byte, path string, structName string) (syncStructField, error) {
	if syncHasNodeType(node, "bitfield_clause") {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("unsupported bitfield in %s (%s) at line %d", path, structName, line)
	}

	typeNode := node.ChildByFieldName("type")
	if typeNode == nil || typeNode.IsNull() {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("field declaration missing type in %s (%s) at line %d", path, structName, line)
	}
	if typeNode.Type() == "struct_specifier" || typeNode.Type() == "union_specifier" {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("unsupported nested declaration in %s (%s) at line %d", path, structName, line)
	}

	ctype := syncNormalizeCType(typeNode.Content(data))
	size, ok := syncCTypeSize(ctype)
	if !ok {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("unsupported c type in %s (%s) at line %d: %q", path, structName, line, ctype)
	}

	decls := syncChildNodesByFieldName(node, "declarator")
	if len(decls) != 1 {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("unsupported multi declarator in %s (%s) at line %d", path, structName, line)
	}

	decl := decls[0]
	if syncHasNodeType(decl, "pointer_declarator") || syncHasNodeType(decl, "array_declarator") || syncHasNodeType(decl, "function_declarator") {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("unsupported field syntax in %s (%s) at line %d", path, structName, line)
	}

	nameNode := syncFindFirstNodeByType(decl, "field_identifier")
	if nameNode == nil || nameNode.IsNull() {
		nameNode = syncFindFirstNodeByType(decl, "identifier")
	}
	if nameNode == nil || nameNode.IsNull() {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("invalid field declarator in %s (%s) at line %d", path, structName, line)
	}
	name := strings.TrimSpace(nameNode.Content(data))
	if !syncFieldNameRegexp.MatchString(name) {
		line := node.StartPoint().Row + 1
		return syncStructField{}, fmt.Errorf("invalid field name in %s (%s) at line %d: %q", path, structName, line, name)
	}

	return syncStructField{name: name, ctype: ctype, size: size}, nil
}

func syncExtractDeclaratorName(node *sitter.Node, data []byte) (string, error) {
	if syncHasNodeType(node, "pointer_type_declarator") || syncHasNodeType(node, "array_declarator") || syncHasNodeType(node, "function_declarator") {
		return "", fmt.Errorf("unsupported typedef declarator %q", node.Type())
	}

	nameNode := syncFindFirstNodeByType(node, "type_identifier")
	if nameNode == nil || nameNode.IsNull() {
		nameNode = syncFindFirstNodeByType(node, "identifier")
	}
	if nameNode == nil || nameNode.IsNull() {
		return "", fmt.Errorf("missing type identifier")
	}
	name := strings.TrimSpace(nameNode.Content(data))
	if !syncFieldNameRegexp.MatchString(name) {
		return "", fmt.Errorf("invalid type identifier %q", name)
	}
	return name, nil
}

func syncFindFirstNodeByType(node *sitter.Node, nodeType string) *sitter.Node {
	if node == nil || node.IsNull() {
		return nil
	}
	if node.Type() == nodeType {
		return node
	}
	for i := 0; i < int(node.ChildCount()); i++ {
		found := syncFindFirstNodeByType(node.Child(i), nodeType)
		if found != nil {
			return found
		}
	}
	return nil
}

func syncHasNodeType(node *sitter.Node, nodeType string) bool {
	return syncFindFirstNodeByType(node, nodeType) != nil
}

func syncChildNodesByFieldName(node *sitter.Node, field string) []*sitter.Node {
	out := make([]*sitter.Node, 0)
	for i := 0; i < int(node.ChildCount()); i++ {
		if node.FieldNameForChild(i) == field {
			out = append(out, node.Child(i))
		}
	}
	return out
}

func syncWalkNode(node *sitter.Node, visit func(*sitter.Node) error) error {
	if node == nil || node.IsNull() {
		return nil
	}
	if err := visit(node); err != nil {
		return err
	}
	for i := 0; i < int(node.ChildCount()); i++ {
		if err := syncWalkNode(node.Child(i), visit); err != nil {
			return err
		}
	}
	return nil
}
