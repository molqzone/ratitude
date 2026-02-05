#ifndef RAT_TYPES_H
#define RAT_TYPES_H

#include <stdint.h>

#define RAT_MAGIC_SIG "RAT_V1"
#define RAT_CHANNEL_COUNT 2

#define RAT_CTX_MAIN 0  // For Main Loop
#define RAT_CTX_ISR 1   // For Interrupt Service Routine

typedef struct
{
  const char* name;      // Channel name (e.g., "Main", "ISR")
  uint8_t* p_buf;        // Pointer to buffer
  uint32_t size;         // Buffer size
  volatile uint32_t wr;  // Write offset (MCU writes)
  volatile uint32_t rd;  // Read offset (Host writes)
  uint32_t flags;        // Reserved flag bits
} RatRingBuffer;

typedef struct
{
  char magic[16];                         // Signature
  RatRingBuffer up[RAT_CHANNEL_COUNT];    // Uplink channels (Device -> Host)
  RatRingBuffer down[RAT_CHANNEL_COUNT];  // Downlink channels (Host -> Device)
} RatControlBlock;

#endif  // RAT_TYPES_H
