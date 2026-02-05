# librat

librat 是一个轻量级、嵌入式友好的 C/C++ 库。

## OpenOCD RTT 模式

librat 仅使用 SEGGER RTT 控制块（符号名：`_SEGGER_RTT`），OpenOCD RTT 可直接读取 COBS 帧数据。

通道映射（无锁 SPSC）：
- Up[0]：`RatMain`，仅主循环写入
- Up[1]：`RatISR`，仅 ISR 写入

缓冲区大小可通过宏配置：
- `RAT_RTT_UP_MAIN_SIZE`
- `RAT_RTT_UP_ISR_SIZE`
- `RAT_RTT_DOWN_BUFFER_SIZE`

典型流程：

1) 通过 ELF 获取控制块地址与大小：

```
arm-none-eabi-nm -S build/stm32f4_rtt/stm32f4_rtt.elf | rg _SEGGER_RTT
```

2) 启动 OpenOCD RTT：

```
openocd -f interface/cmsis-dap.cfg -f target/stm32f4x.cfg \
  -c "init" -c "reset init" \
  -c "rtt setup <addr> <size> \"SEGGER RTT\"" -c "rtt start" \
  -c "rtt server start 19021 0"
```

3) Host 侧读取（默认端口 19021）：

```
rttd server --addr 127.0.0.1:19021 --log out.jsonl
```
