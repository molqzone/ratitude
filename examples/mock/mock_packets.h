#pragma once

#include <stdint.h>

// @rat, quat
typedef struct {
  float x;
  float y;
  float z;
  float w;
} Attitude;

// @rat, plot
typedef struct {
  float celsius;
  uint32_t tick_ms;
} Temperature;

// @rat, plot
typedef struct {
  float value;
  uint32_t tick_ms;
} Waveform;

// @rat, image
typedef struct __attribute__((packed)) {
  uint16_t width;
  uint16_t height;
  uint32_t frame_idx;
  uint8_t luma;
} ImageStats;
