# @rat 声明规范（当前实现）

## 1. 语法

支持两种注释：

```c
// @rat, plot
/* @rat, quat */
```

也支持省略类型：

```c
// @rat
```

省略时默认 `plot`。

## 2. 类型集合

当前允许：

- `plot`（默认）
- `quat`
- `image`
- `log`

## 3. 已废弃语法（会报错）

以下写法不再兼容：

- `@rat:id=0x01, ...`
- `@rat, type=plot`

## 4. 结构约束（当前 parser）

要求：

- `@rat` 后必须匹配一个后续 `typedef struct`
- 字段必须是可识别的基础类型（如 `int32_t`、`float`）

限制：

- 不支持 bitfield
- 不支持指针 / 数组 / 函数声明字段
- 不支持嵌套 `struct` / `union` 字段

## 5. 推荐写法

```c
// @rat, quat
typedef struct {
  float x;
  float y;
  float z;
  float w;
} AttitudePacket;
```

## 6. Foxglove 映射（破坏式更新）

- `rttd foxglove` 不再接受类型映射相关 CLI 参数（如 `--quat-id`、`--topic`）
- 所有 Foxglove 通道与 schema 仅由 `rat_gen.toml` 的声明结果决定
- 每个声明包发布到 `/rat/{struct_name}`，schema 为 `ratitude.{struct_name}`
- `quat` 类型额外发布 `/rat/{struct_name}/marker` 与 `/rat/{struct_name}/tf`

## 7. 与 Mock 联调的关系

- `tools/openocd_rtt_mock.py` 只读取 `rat_gen.toml` 中声明过的包
- mock 不会内置“额外默认包”或隐藏 ID
- `image` 包会派生 `/rat/{struct_name}/image` 图像通道（foxglove.RawImage）
- mock 使用声明字段（如 `width/height/frame_idx/luma`）生成可视化动态图像
