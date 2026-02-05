#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "rat_internal.h"

static uint8_t g_rtt_up_main[RAT_RTT_UP_MAIN_SIZE];
static uint8_t g_rtt_up_isr[RAT_RTT_UP_ISR_SIZE];
static uint8_t g_rtt_down[RAT_RTT_DOWN_BUFFER_SIZE];

RatRttControlBlock _SEGGER_RTT;

static const char k_rtt_id[] = RAT_RTT_ID;
static const char k_rtt_name_main[] = "RatMain";
static const char k_rtt_name_isr[] = "RatISR";
static const char k_rtt_name_down[] = "RatDown";

static uint32_t rat_rtt_ring_used(const RatRttRingBuffer* rb)
{
  uint32_t wr = rb->wr;
  uint32_t rd = rb->rd;

  if (wr >= rd)
  {
    return wr - rd;
  }

  return rb->size - (rd - wr);
}

static uint32_t rat_rtt_ring_free(const RatRttRingBuffer* rb)
{
  uint32_t used = rat_rtt_ring_used(rb);
  if (rb->size == 0u)
  {
    return 0u;
  }
  return rb->size - used - 1u;
}

void rat_rtt_init(void)
{
  memset(&_SEGGER_RTT, 0, sizeof(_SEGGER_RTT));
  memset(g_rtt_up_main, 0, sizeof(g_rtt_up_main));
  memset(g_rtt_up_isr, 0, sizeof(g_rtt_up_isr));
  memset(g_rtt_down, 0, sizeof(g_rtt_down));

  memcpy(_SEGGER_RTT.id, k_rtt_id, sizeof(k_rtt_id) - 1u);
  _SEGGER_RTT.max_up = RAT_RTT_UP_COUNT;
  _SEGGER_RTT.max_down = RAT_RTT_DOWN_COUNT;

  _SEGGER_RTT.up[RAT_CTX_MAIN].sName = k_rtt_name_main;
  _SEGGER_RTT.up[RAT_CTX_MAIN].pBuffer = (char*)g_rtt_up_main;
  _SEGGER_RTT.up[RAT_CTX_MAIN].size = (uint32_t)sizeof(g_rtt_up_main);
  _SEGGER_RTT.up[RAT_CTX_MAIN].wr = 0u;
  _SEGGER_RTT.up[RAT_CTX_MAIN].rd = 0u;
  _SEGGER_RTT.up[RAT_CTX_MAIN].flags = 0u;

  _SEGGER_RTT.up[RAT_CTX_ISR].sName = k_rtt_name_isr;
  _SEGGER_RTT.up[RAT_CTX_ISR].pBuffer = (char*)g_rtt_up_isr;
  _SEGGER_RTT.up[RAT_CTX_ISR].size = (uint32_t)sizeof(g_rtt_up_isr);
  _SEGGER_RTT.up[RAT_CTX_ISR].wr = 0u;
  _SEGGER_RTT.up[RAT_CTX_ISR].rd = 0u;
  _SEGGER_RTT.up[RAT_CTX_ISR].flags = 0u;

  _SEGGER_RTT.down[0].sName = k_rtt_name_down;
  _SEGGER_RTT.down[0].pBuffer = (char*)g_rtt_down;
  _SEGGER_RTT.down[0].size = (uint32_t)sizeof(g_rtt_down);
  _SEGGER_RTT.down[0].wr = 0u;
  _SEGGER_RTT.down[0].rd = 0u;
  _SEGGER_RTT.down[0].flags = 0u;
}

int rat_rtt_write(uint8_t packet_id, const void* data, uint32_t len, bool in_isr)
{
  RatRttRingBuffer* rb = &_SEGGER_RTT.up[in_isr ? RAT_CTX_ISR : RAT_CTX_MAIN];
  const uint8_t* bytes = (const uint8_t*)data;
  size_t raw_len = (size_t)len + 1u;
  size_t needed = rat_cobs_max_encoded_length(raw_len);

  if (rb->size == 0u || rb->pBuffer == NULL)
  {
    return 0;
  }

  if ((size_t)rat_rtt_ring_free(rb) < needed)
  {
    return 0;
  }

  RatCobsState state;
  rat_cobs_begin(&state, rb, rb->wr);
  rat_cobs_write_byte(&state, rb, packet_id);

  for (uint32_t i = 0u; i < len; ++i)
  {
    rat_cobs_write_byte(&state, rb, bytes[i]);
  }

  rat_cobs_finish(&state, rb);

  RAT_MEM_BARRIER();
  rb->wr = state.wr;
  RAT_MEM_BARRIER();

  return (int)state.written;
}

#ifdef RAT_INTERNAL_TEST
void rat_internal_get_rtt_state(uint8_t channel,
                                const uint8_t** buffer,
                                uint32_t* size,
                                uint32_t* wr,
                                uint32_t* rd)
{
  if (channel >= RAT_RTT_UP_COUNT)
  {
    return;
  }
  if (buffer != NULL)
  {
    *buffer = (const uint8_t*)_SEGGER_RTT.up[channel].pBuffer;
  }
  if (size != NULL)
  {
    *size = _SEGGER_RTT.up[channel].size;
  }
  if (wr != NULL)
  {
    *wr = _SEGGER_RTT.up[channel].wr;
  }
  if (rd != NULL)
  {
    *rd = _SEGGER_RTT.up[channel].rd;
  }
}
#endif
