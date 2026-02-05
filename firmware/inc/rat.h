#ifndef RAT_H
#define RAT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "rat_types.h"

#ifdef __cplusplus
extern "C"
{
#endif

  /**
   * @brief Initialize the Ratitude library
   * Must be called early in `main()`.
   * Initializes the control block, mounts buffers, and enables host discovery.
   */
  void rat_init(void);

  /**
   * @brief Send a binary packet
   *
   * Thread-safety: call only from the main loop or low-priority threads.
   * Do NOT call from interrupts (ISRs); this may cause data races.
   *
   * Features:
   * - Performs COBS encoding (framing).
   * - Atomic delivery: Host will receive either the complete packet or none (no partial
   * packets).
   * - Lock-free: uses only memory barriers.
   *
   * @param packet_id User-defined ID (0x00-0xFF) used by the Host to distinguish struct
   * types.
   * @param data      Pointer to the struct.
   * @param len       Size of the struct.
   * @param in_isr    Set to true if called from an ISR context.W
   * @return int      Number of bytes actually written (including overhead). Returns 0 if
   * the buffer is full.
   */
  int rat_emit(uint8_t packet_id, const void* data, uint32_t len, bool in_isr);

  /**
   * @brief Send a text log (printf-style)
   * Defaults to the Main channel.
   */
  void rat_info(const char* fmt, ...);

/* Macro that automatically calculates struct size for convenience */
#define RAT_EMIT(id, obj) rat_emit(id, &(obj), sizeof(obj), false)

#define RAT_EMIT_ISR(id, obj) rat_emit(id, &(obj), sizeof(obj), true)

#ifdef __cplusplus
}
#endif

#endif  // RAT_H
