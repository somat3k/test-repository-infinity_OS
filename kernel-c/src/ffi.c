/**
 * @file ffi.c
 * @brief infinityOS Kernel — ABI-Stable FFI Export Surface
 *
 * Implements the ABI negotiation, dimension management, and TaskID
 * generation functions declared in ffi.h.
 *
 * TaskIDs use a 128-bit layout compatible with UUID v7:
 *   hi: millisecond Unix timestamp (48 bits) | version (4 bits) | random_a (12 bits)
 *   lo: variant (2 bits) | random_b (62 bits)
 *
 * For this implementation we use the monotonic kernel time (not wall-clock)
 * as the high-word timestamp to guarantee monotonicity within a dimension.
 * The low word is a monotonic per-dimension counter.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

/* Needed for snprintf and inttypes on POSIX systems. */
#define _POSIX_C_SOURCE 200809L

#include <string.h>
#include <stdio.h>
#include <stdint.h>
#include <inttypes.h>

#include "internal.h"
#include "../include/infinity/ffi.h"

/* --------------------------------------------------------------------------
 * Dimension table
 * ------------------------------------------------------------------------ */

#define DIM_TABLE_MAX 1024u

/** Per-dimension state stored in the global table. */
typedef struct {
    ify_dimension_id_t id;        /**< Non-zero when slot is occupied. */
    uint64_t           task_seq;  /**< Per-dimension task sequence.    */
} dim_slot_t;

/** Global dimension table protected by the kernel lock. */
static dim_slot_t g_dims[DIM_TABLE_MAX];

/** Find a dimension slot by ID (caller holds lock). */
static dim_slot_t *dim_find(ify_dimension_id_t id) {
    for (uint32_t i = 0; i < DIM_TABLE_MAX; i++) {
        if (g_dims[i].id == id) {
            return &g_dims[i];
        }
    }
    return NULL;
}

/** Find a free dimension slot (caller holds lock). */
static dim_slot_t *dim_alloc_slot(void) {
    for (uint32_t i = 0; i < DIM_TABLE_MAX; i++) {
        if (g_dims[i].id == 0) {
            return &g_dims[i];
        }
    }
    return NULL;
}

/* --------------------------------------------------------------------------
 * ify_ffi_abi_info
 * ------------------------------------------------------------------------ */

ify_status_t ify_ffi_abi_info(ify_ffi_abi_t *out) {
    if (out == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    IFY_REQUIRE_INIT();

    memset(out, 0, sizeof(*out));
    out->version        = INFINITY_KERNEL_VERSION;
    out->struct_size    = (uint32_t)sizeof(ify_ffi_abi_t);
    out->caps_available = g_kernel.granted_caps;
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_dimension_create / ify_dimension_destroy
 * ------------------------------------------------------------------------ */

ify_status_t ify_dimension_create(ify_dimension_id_t *out_id) {
    if (out_id == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    IFY_REQUIRE_INIT();

    pthread_mutex_lock(&g_kernel.lock);

    dim_slot_t *slot = dim_alloc_slot();
    if (slot == NULL) {
        pthread_mutex_unlock(&g_kernel.lock);
        return IFY_ERR_OVERFLOW;
    }

    ify_dimension_id_t new_id =
        atomic_fetch_add_explicit(&g_kernel.dim_counter, 1, memory_order_relaxed);
    slot->id       = new_id;
    slot->task_seq = 0;
    *out_id = new_id;

    pthread_mutex_unlock(&g_kernel.lock);
    return IFY_OK;
}

ify_status_t ify_dimension_destroy(ify_dimension_id_t id) {
    if (id == 0) {
        return IFY_ERR_INVALID_ARG;
    }
    IFY_REQUIRE_INIT();

    pthread_mutex_lock(&g_kernel.lock);

    dim_slot_t *slot = dim_find(id);
    if (slot == NULL) {
        pthread_mutex_unlock(&g_kernel.lock);
        return IFY_ERR_NOT_FOUND;
    }
    memset(slot, 0, sizeof(*slot));

    pthread_mutex_unlock(&g_kernel.lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_task_id_generate
 * ------------------------------------------------------------------------ */

ify_status_t ify_task_id_generate(ify_dimension_id_t dimension_id,
                                   ify_task_id_t *out) {
    if (out == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    IFY_REQUIRE_INIT();

    pthread_mutex_lock(&g_kernel.lock);

    dim_slot_t *slot = dim_find(dimension_id);
    if (slot == NULL) {
        pthread_mutex_unlock(&g_kernel.lock);
        return IFY_ERR_NOT_FOUND;
    }

    uint64_t seq = ++slot->task_seq;
    uint64_t ts  = ify_time_now_ns();

    /* Construct a UUID-v7-inspired layout.
     *   hi: timestamp (upper 48 bits) | 0x7 (version, bits 15:12) | seq_hi (bits 11:0)
     *   lo: 0b10 (variant, bits 63:62) | seq_lo (bits 61:0)
     */
    out->hi = (ts & UINT64_C(0xFFFFFFFFFFFF0000)) |
              UINT64_C(0x7000) |
              (seq >> 16 & UINT64_C(0x0FFF));
    out->lo = UINT64_C(0x8000000000000000) | (seq & UINT64_C(0x3FFFFFFFFFFFFFFF));

    pthread_mutex_unlock(&g_kernel.lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_task_id_to_str
 * ------------------------------------------------------------------------ */

void ify_task_id_to_str(ify_task_id_t id, char buf[37]) {
    if (buf == NULL) {
        return;
    }
    /* Format as xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx */
    uint32_t p0 = (uint32_t)(id.hi >> 32);
    uint32_t p1 = (uint32_t)((id.hi >> 16) & 0xFFFFu);
    uint32_t p2 = (uint32_t)(id.hi & 0xFFFFu);
    uint32_t p3 = (uint32_t)(id.lo >> 48);
    uint64_t p4 = id.lo & UINT64_C(0x0000FFFFFFFFFFFF);

    snprintf(buf, 37, "%08" PRIx32 "-%04" PRIx32 "-%04" PRIx32
                      "-%04" PRIx32 "-%012" PRIx64,
             p0, p1, p2, p3, p4);
}
