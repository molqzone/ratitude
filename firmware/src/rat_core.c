#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "rat.h"
#include "rat_internal.h"

static RatControlBlock g_cb;

static uint8_t g_up_main[RAT_UP_BUFFER_SIZE];
static uint8_t g_up_isr[RAT_UP_BUFFER_SIZE];
static uint8_t g_down_main[RAT_DOWN_BUFFER_SIZE];
static uint8_t g_down_isr[RAT_DOWN_BUFFER_SIZE];

static const char k_name_main[] = "Main";
static const char k_name_isr[] = "ISR";

static uint32_t rat_ring_used(const RatRingBuffer* rb)
{
  uint32_t wr = rb->wr;
  uint32_t rd = rb->rd;

  if (wr >= rd)
  {
    return wr - rd;
  }

  return rb->size - (rd - wr);
}

static uint32_t rat_ring_free(const RatRingBuffer* rb)
{
  uint32_t used = rat_ring_used(rb);
  if (rb->size == 0u)
  {
    return 0u;
  }
  return rb->size - used - 1u;
}

void rat_init(void)
{
  memset(&g_cb, 0, sizeof(g_cb));
  memset(g_cb.magic, 0, sizeof(g_cb.magic));
  memcpy(g_cb.magic, RAT_MAGIC_SIG, sizeof(RAT_MAGIC_SIG) - 1u);

  g_cb.up[RAT_CTX_MAIN].name = k_name_main;
  g_cb.up[RAT_CTX_MAIN].p_buf = g_up_main;
  g_cb.up[RAT_CTX_MAIN].size = (uint32_t)sizeof(g_up_main);
  g_cb.up[RAT_CTX_MAIN].wr = 0u;
  g_cb.up[RAT_CTX_MAIN].rd = 0u;
  g_cb.up[RAT_CTX_MAIN].flags = 0u;

  g_cb.up[RAT_CTX_ISR].name = k_name_isr;
  g_cb.up[RAT_CTX_ISR].p_buf = g_up_isr;
  g_cb.up[RAT_CTX_ISR].size = (uint32_t)sizeof(g_up_isr);
  g_cb.up[RAT_CTX_ISR].wr = 0u;
  g_cb.up[RAT_CTX_ISR].rd = 0u;
  g_cb.up[RAT_CTX_ISR].flags = 0u;

  g_cb.down[RAT_CTX_MAIN].name = k_name_main;
  g_cb.down[RAT_CTX_MAIN].p_buf = g_down_main;
  g_cb.down[RAT_CTX_MAIN].size = (uint32_t)sizeof(g_down_main);
  g_cb.down[RAT_CTX_MAIN].wr = 0u;
  g_cb.down[RAT_CTX_MAIN].rd = 0u;
  g_cb.down[RAT_CTX_MAIN].flags = 0u;

  g_cb.down[RAT_CTX_ISR].name = k_name_isr;
  g_cb.down[RAT_CTX_ISR].p_buf = g_down_isr;
  g_cb.down[RAT_CTX_ISR].size = (uint32_t)sizeof(g_down_isr);
  g_cb.down[RAT_CTX_ISR].wr = 0u;
  g_cb.down[RAT_CTX_ISR].rd = 0u;
  g_cb.down[RAT_CTX_ISR].flags = 0u;
}

int rat_emit(uint8_t packet_id, const void* data, uint32_t len, bool in_isr)
{
  RatRingBuffer* rb = &g_cb.up[in_isr ? RAT_CTX_ISR : RAT_CTX_MAIN];
  const uint8_t* bytes = (const uint8_t*)data;
  size_t raw_len = (size_t)len + 1u;
  size_t needed = rat_cobs_max_encoded_length(raw_len);

  if (rb->size == 0u)
  {
    return 0;
  }

  if ((size_t)rat_ring_free(rb) < needed)
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

void rat_info(const char* fmt, ...)
{
  char buffer[RAT_INFO_MAX_LEN];
  va_list args;
  va_start(args, fmt);
  int written = vsnprintf(buffer, sizeof(buffer), fmt, args);
  va_end(args);

  if (written <= 0)
  {
    return;
  }

  if ((size_t)written >= sizeof(buffer))
  {
    written = (int)(sizeof(buffer) - 1u);
    buffer[written] = '\0';
  }

  (void)rat_emit(RAT_TEXT_PACKET_ID, buffer, (uint32_t)written, false);
}

#ifdef RAT_INTERNAL_TEST
RatControlBlock* rat_internal_get_cb(void) { return &g_cb; }
#endif
