#ifndef RAT_INTERNAL_H
#define RAT_INTERNAL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "rat_types.h"

#ifdef __cplusplus
extern "C" {
#endif

#ifndef RAT_INFO_MAX_LEN
#define RAT_INFO_MAX_LEN 128u
#endif

#ifndef RAT_TEXT_PACKET_ID
#define RAT_TEXT_PACKET_ID 0xFFu
#endif

#ifndef RAT_RTT_UP_MAIN_SIZE
#define RAT_RTT_UP_MAIN_SIZE 1024u
#endif

#ifndef RAT_RTT_UP_ISR_SIZE
#define RAT_RTT_UP_ISR_SIZE 1024u
#endif

#ifndef RAT_RTT_DOWN_BUFFER_SIZE
#define RAT_RTT_DOWN_BUFFER_SIZE 16u
#endif

#ifndef RAT_RTT_UP_BUFFER_SIZE
#define RAT_RTT_UP_BUFFER_SIZE 1024u
#endif

#ifndef RAT_RTT_DOWN_BUFFER_SIZE
#define RAT_RTT_DOWN_BUFFER_SIZE 16u
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

void rat_cobs_begin(RatCobsState* state, RatRttRingBuffer* rb, uint32_t wr);
void rat_cobs_write_byte(RatCobsState* state, RatRttRingBuffer* rb, uint8_t byte);
void rat_cobs_finish(RatCobsState* state, RatRttRingBuffer* rb);

void rat_rtt_init(void);
int rat_rtt_write(uint8_t packet_id, const void* data, uint32_t len, bool in_isr);

#ifdef RAT_INTERNAL_TEST
void rat_internal_get_rtt_state(uint8_t channel,
                                const uint8_t** buffer,
                                uint32_t* size,
                                uint32_t* wr,
                                uint32_t* rd);
#endif

#ifdef __cplusplus
}
#endif

#endif  // RAT_INTERNAL_H
