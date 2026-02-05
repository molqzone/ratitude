#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#include "rat.h"
#include "rat_internal.h"

void rat_init(void)
{
  rat_rtt_init();
}

int rat_emit(uint8_t packet_id, const void* data, uint32_t len, bool in_isr)
{
  return rat_rtt_write(packet_id, data, len, in_isr);
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
#endif
