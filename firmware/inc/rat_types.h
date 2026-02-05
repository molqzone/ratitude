#ifndef RAT_TYPES_H
#define RAT_TYPES_H

#include <stdint.h>

#define RAT_RTT_ID "SEGGER RTT"
#define RAT_RTT_UP_COUNT 2
#define RAT_RTT_DOWN_COUNT 1

#define RAT_CTX_MAIN 0  // For Main Loop
#define RAT_CTX_ISR 1   // For Interrupt Service Routine

typedef struct
{
  const char* sName;
  char* pBuffer;
  uint32_t size;
  volatile uint32_t wr;
  volatile uint32_t rd;
  uint32_t flags;
} RatRttRingBuffer;

typedef struct
{
  char id[16];
  int max_up;
  int max_down;
  RatRttRingBuffer up[RAT_RTT_UP_COUNT];
  RatRttRingBuffer down[RAT_RTT_DOWN_COUNT];
} RatRttControlBlock;

extern RatRttControlBlock _SEGGER_RTT;

#endif  // RAT_TYPES_H
