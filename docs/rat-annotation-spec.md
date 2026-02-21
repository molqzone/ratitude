# @rat 声明规范

## 1. 语法

支持：

- `// @rat, plot`
- `// @rat, quat`
- `// @rat, image`
- `// @rat, log`
- `// @rat`（默认 `plot`）

示例：

```c
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick_ms;
} RatSample;

// @rat, quat
typedef struct {
  float x;
  float y;
  float z;
  float w;
} AttitudeQuat;
```

## 2. 限制

- 不支持旧语法 `@rat:id=...` 与 `@rat, type=...`
- 不支持 `aligned(...)` / `#pragma pack`
- 非 `packed` 且存在 padding / 8 字节字段风险会被阻断

## 3. 生成结果

声明会被内部同步逻辑转换为：

- `rat_gen.toml`（主机解码）
- `rat_gen.h`（固件 ID 与指纹）

## 4. 输出行为

- `quat` 类型可映射到 Foxglove 四元数相关输出
- `image` 类型会派生 `/rat/{struct_name}/image` 图像通道（derived image）
