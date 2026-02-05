#ifndef RAT_INTERNAL_H
#define RAT_INTERNAL_H

#include <stddef.h>
#include <stdint.h>

#include "rat_types.h"

#ifdef __cplusplus
extern "C" {
#endif

#ifndef RAT_UP_BUFFER_SIZE
#define RAT_UP_BUFFER_SIZE 1024u
#endif

#ifndef RAT_DOWN_BUFFER_SIZE
#define RAT_DOWN_BUFFER_SIZE 256u
#endif

#ifndef RAT_INFO_MAX_LEN
#define RAT_INFO_MAX_LEN 128u
#endif

#ifndef RAT_TEXT_PACKET_ID
#define RAT_TEXT_PACKET_ID 0xFFu
#endif

#ifndef RAT_MEM_BARRIER
#if defined(__GNUC__) || defined(__clang__)
#define RAT_MEM_BARRIER() __asm__ volatile("" ::: "memory")
#else
#define RAT_MEM_BARRIER() do { } while (0)
#endif
#endif

size_t rat_cobs_max_encoded_length(size_t input_len);

typedef struct
{
  uint32_t code_pos;
  uint8_t code;
  uint32_t wr;
  size_t written;
} RatCobsState;

void rat_cobs_begin(RatCobsState* state, RatRingBuffer* rb, uint32_t wr);
void rat_cobs_write_byte(RatCobsState* state, RatRingBuffer* rb, uint8_t byte);
void rat_cobs_finish(RatCobsState* state, RatRingBuffer* rb);

#ifdef RAT_INTERNAL_TEST
RatControlBlock* rat_internal_get_cb(void);
#endif

#ifdef __cplusplus
}
#endif

#endif  // RAT_INTERNAL_H
