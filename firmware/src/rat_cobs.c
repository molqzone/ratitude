#include <stddef.h>
#include <stdint.h>

#include "rat_internal.h"

size_t rat_cobs_max_encoded_length(size_t input_len)
{
  size_t overhead = (input_len / 254u) + 1u;
  return input_len + overhead + 1u;  // +1 for delimiter 0x00
}

void rat_cobs_begin(RatCobsState* state, RatRingBuffer* rb, uint32_t wr)
{
  state->code_pos = wr;
  rb->p_buf[wr] = 0u;
  state->wr = (wr + 1u) % rb->size;
  state->code = 1u;
  state->written = 1u;
}

void rat_cobs_write_byte(RatCobsState* state, RatRingBuffer* rb, uint8_t byte)
{
  if (byte == 0u)
  {
    rb->p_buf[state->code_pos] = state->code;
    state->code_pos = state->wr;
    rb->p_buf[state->wr] = 0u;
    state->wr = (state->wr + 1u) % rb->size;
    state->code = 1u;
    state->written += 1u;
    return;
  }

  rb->p_buf[state->wr] = byte;
  state->wr = (state->wr + 1u) % rb->size;
  state->code += 1u;
  state->written += 1u;

  if (state->code == 0xFFu)
  {
    rb->p_buf[state->code_pos] = state->code;
    state->code_pos = state->wr;
    rb->p_buf[state->wr] = 0u;
    state->wr = (state->wr + 1u) % rb->size;
    state->code = 1u;
    state->written += 1u;
  }
}

void rat_cobs_finish(RatCobsState* state, RatRingBuffer* rb)
{
  rb->p_buf[state->code_pos] = state->code;
  rb->p_buf[state->wr] = 0u;
  state->wr = (state->wr + 1u) % rb->size;
  state->written += 1u;
}
