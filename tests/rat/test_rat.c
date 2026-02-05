#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "rat.h"
#include "rat_internal.h"

static size_t cobs_decode(const uint8_t* input, size_t len, uint8_t* output, size_t out_cap)
{
  size_t in = 0u;
  size_t out = 0u;

  while (in < len)
  {
    uint8_t code = input[in++];
    if (code == 0u)
    {
      break;
    }

    for (uint8_t i = 1u; i < code && in < len; ++i)
    {
      if (out < out_cap)
      {
        output[out++] = input[in];
      }
      in += 1u;
    }

    if (code != 0xFFu && in < len && input[in] != 0u)
    {
      if (out < out_cap)
      {
        output[out++] = 0u;
      }
    }
  }

  return out;
}

static void test_emit_roundtrip(void)
{
  uint8_t payload[3] = {0x11u, 0x00u, 0x22u};
  rat_init();

  int written = rat_emit(0x42u, payload, (uint32_t)sizeof(payload), false);
  assert(written > 0);

  const uint8_t* buffer = NULL;
  uint32_t size = 0u;
  uint32_t wr = 0u;
  rat_internal_get_rtt_state(RAT_CTX_MAIN, &buffer, &size, &wr, NULL);
  assert(buffer != NULL);
  assert(size > 0u);

  uint8_t decoded[8] = {0};
  size_t decoded_len = cobs_decode(buffer, wr, decoded, sizeof(decoded));

  assert(decoded_len == 1u + sizeof(payload));
  assert(decoded[0] == 0x42u);
  assert(memcmp(&decoded[1], payload, sizeof(payload)) == 0);
}

static void test_emit_full(void)
{
  rat_init();

  uint8_t data[4] = {1u, 2u, 3u, 4u};
  int written = 0;

  const uint8_t* buffer = NULL;
  uint32_t size = 0u;
  rat_internal_get_rtt_state(RAT_CTX_MAIN, &buffer, &size, NULL, NULL);
  assert(size > 0u);

  uint32_t attempts = size * 2u;
  for (uint32_t i = 0u; i < attempts; ++i)
  {
    written = rat_emit(0x01u, data, (uint32_t)sizeof(data), false);
    if (written == 0)
    {
      return;
    }
  }

  assert(written == 0);
}

static void test_info_packet(void)
{
  rat_init();

  rat_info("ok");

  uint32_t wr = 0u;
  rat_internal_get_rtt_state(RAT_CTX_MAIN, NULL, NULL, &wr, NULL);
  assert(wr > 0u);
}

static void test_emit_rtt_channels(void)
{
  uint8_t payload[2] = {0x11u, 0x22u};
  rat_init();

  int written = rat_emit(0x7Au, payload, (uint32_t)sizeof(payload), false);
  assert(written > 0);

  uint32_t wr_main = 0u;
  uint32_t wr_isr = 0u;
  rat_internal_get_rtt_state(RAT_CTX_MAIN, NULL, NULL, &wr_main, NULL);
  rat_internal_get_rtt_state(RAT_CTX_ISR, NULL, NULL, &wr_isr, NULL);

  assert(wr_main > 0u);
  assert(wr_isr == 0u);

  uint8_t isr_payload[2] = {0x33u, 0x44u};
  written = rat_emit(0x7Bu, isr_payload, (uint32_t)sizeof(isr_payload), true);
  assert(written > 0);

  rat_internal_get_rtt_state(RAT_CTX_ISR, NULL, NULL, &wr_isr, NULL);
  assert(wr_isr > 0u);
}

int main(void)
{
  test_emit_roundtrip();
  test_emit_full();
  test_info_packet();
  test_emit_rtt_channels();
  return 0;
}
