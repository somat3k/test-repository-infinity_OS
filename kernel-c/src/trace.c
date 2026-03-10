/**
 * @file trace.c
 * @brief infinityOS Kernel — Tracing Hooks and Telemetry Implementation
 *
 * Ring-buffer-based span collector.  In-flight spans are tracked in a
 * separate hash map (array indexed by span_id % IFY_TRACE_INFLIGHT_MAX)
 * so that ify_trace_begin() and ify_trace_end() do not have to scan the
 * ring buffer.
 *
 * The ring buffer uses a power-of-two capacity; write_idx and read_idx
 * are monotonic counters and the buffer slot is derived with & (cap-1).
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "internal.h"
#include "../include/infinity/trace.h"

/* --------------------------------------------------------------------------
 * In-flight span tracking
 * ------------------------------------------------------------------------ */

#define IFY_TRACE_INFLIGHT_MAX 256u

typedef struct {
    uint64_t       span_id;
    ify_span_t     partial;   /**< Partially-filled span (no end_ns yet). */
    int            in_use;
} inflight_entry_t;

/* --------------------------------------------------------------------------
 * Trace state
 * ------------------------------------------------------------------------ */

typedef struct {
    pthread_mutex_t      lock;
    ify_span_t          *ring;          /**< Heap-allocated ring buffer.      */
    uint32_t             cap;           /**< Capacity (power of two).         */
    uint64_t             write_idx;     /**< Monotonic write counter.         */
    uint64_t             read_idx;      /**< Monotonic read counter.          */
    atomic_uint_fast64_t span_seq;      /**< Monotonic span ID source.        */
    ify_trace_emit_fn_t  emit_fn;
    void                *emit_ctx;
    inflight_entry_t     inflight[IFY_TRACE_INFLIGHT_MAX];
    int                  initialized;
} trace_state_t;

static trace_state_t g_trace = {
    .lock         = PTHREAD_MUTEX_INITIALIZER,
    .ring         = NULL,
    .cap          = 0,
    .write_idx    = 0,
    .read_idx     = 0,
    .span_seq     = ATOMIC_VAR_INIT(0),
    .emit_fn      = NULL,
    .emit_ctx     = NULL,
    .initialized  = 0,
};

/* --------------------------------------------------------------------------
 * ify_trace_init / ify_trace_shutdown
 * ------------------------------------------------------------------------ */

ify_status_t ify_trace_init(uint32_t capacity) {
    /* Capacity must be a power of two >= 2. */
    if (capacity < 2 || (capacity & (capacity - 1)) != 0) {
        return IFY_ERR_INVALID_ARG;
    }

    pthread_mutex_lock(&g_trace.lock);
    if (g_trace.initialized) {
        pthread_mutex_unlock(&g_trace.lock);
        return IFY_ERR_ALREADY_EXISTS;
    }

    g_trace.ring = (ify_span_t *)calloc(capacity, sizeof(ify_span_t));
    if (g_trace.ring == NULL) {
        pthread_mutex_unlock(&g_trace.lock);
        return IFY_ERR_OUT_OF_MEMORY;
    }

    g_trace.cap         = capacity;
    g_trace.write_idx   = 0;
    g_trace.read_idx    = 0;
    g_trace.initialized = 1;
    memset(g_trace.inflight, 0, sizeof(g_trace.inflight));

    pthread_mutex_unlock(&g_trace.lock);
    return IFY_OK;
}

void ify_trace_shutdown(void) {
    pthread_mutex_lock(&g_trace.lock);
    if (!g_trace.initialized) {
        pthread_mutex_unlock(&g_trace.lock);
        return;
    }
    free(g_trace.ring);
    g_trace.ring        = NULL;
    g_trace.cap         = 0;
    g_trace.initialized = 0;
    g_trace.emit_fn     = NULL;
    g_trace.emit_ctx    = NULL;
    pthread_mutex_unlock(&g_trace.lock);
}

/* --------------------------------------------------------------------------
 * ify_trace_set_emit
 * ------------------------------------------------------------------------ */

void ify_trace_set_emit(ify_trace_emit_fn_t fn, void *ctx) {
    pthread_mutex_lock(&g_trace.lock);
    g_trace.emit_fn  = fn;
    g_trace.emit_ctx = ctx;
    pthread_mutex_unlock(&g_trace.lock);
}

/* --------------------------------------------------------------------------
 * ify_trace_begin
 * ------------------------------------------------------------------------ */

uint64_t ify_trace_begin(uint64_t           parent_span_id,
                          ify_dimension_id_t dimension_id,
                          ify_task_id_t      task_id,
                          const char        *op) {
    uint64_t sid = atomic_fetch_add_explicit(&g_trace.span_seq, 1,
                                             memory_order_relaxed) + 1;

    pthread_mutex_lock(&g_trace.lock);
    if (!g_trace.initialized) {
        pthread_mutex_unlock(&g_trace.lock);
        return sid;
    }

    /* Store as in-flight entry. */
    uint32_t slot = (uint32_t)(sid % IFY_TRACE_INFLIGHT_MAX);
    inflight_entry_t *e = &g_trace.inflight[slot];

    memset(e, 0, sizeof(*e));
    e->span_id              = sid;
    e->in_use               = 1;
    e->partial.span_id      = sid;
    e->partial.parent_span_id = parent_span_id;
    e->partial.dimension_id = dimension_id;
    e->partial.task_id      = task_id;
    e->partial.start_ns     = ify_time_now_ns();

    if (op != NULL) {
        size_t len = 0;
        while (op[len] != '\0' && len < IFY_TRACE_OP_MAX - 1) {
            len++;
        }
        memcpy(e->partial.op, op, len);
        e->partial.op[len] = '\0';
    }

    pthread_mutex_unlock(&g_trace.lock);
    return sid;
}

/* --------------------------------------------------------------------------
 * ify_trace_end
 * ------------------------------------------------------------------------ */

void ify_trace_end(uint64_t span_id, uint64_t alloc_bytes, uint64_t free_bytes) {
    if (span_id == 0) {
        return;
    }

    pthread_mutex_lock(&g_trace.lock);
    if (!g_trace.initialized) {
        pthread_mutex_unlock(&g_trace.lock);
        return;
    }

    uint32_t slot = (uint32_t)(span_id % IFY_TRACE_INFLIGHT_MAX);
    inflight_entry_t *e = &g_trace.inflight[slot];

    if (!e->in_use || e->span_id != span_id) {
        /* Span not found (collision or already ended). */
        pthread_mutex_unlock(&g_trace.lock);
        return;
    }

    e->partial.end_ns      = ify_time_now_ns();
    e->partial.alloc_bytes = alloc_bytes;
    e->partial.free_bytes  = free_bytes;

    /* Write to ring buffer (overwrite oldest if full). */
    uint32_t idx = (uint32_t)(g_trace.write_idx & (uint64_t)(g_trace.cap - 1));
    g_trace.ring[idx] = e->partial;
    g_trace.write_idx++;

    /* If buffer is full, advance read pointer (oldest span lost). */
    if (g_trace.write_idx - g_trace.read_idx > (uint64_t)g_trace.cap) {
        g_trace.read_idx = g_trace.write_idx - (uint64_t)g_trace.cap;
    }

    /* Invoke emit callback before releasing the lock. */
    ify_trace_emit_fn_t fn  = g_trace.emit_fn;
    void               *ctx = g_trace.emit_ctx;
    ify_span_t          span_copy = e->partial;

    e->in_use = 0;
    pthread_mutex_unlock(&g_trace.lock);

    if (fn != NULL) {
        fn(&span_copy, ctx);
    }
}

/* --------------------------------------------------------------------------
 * ify_trace_read
 * ------------------------------------------------------------------------ */

uint32_t ify_trace_read(ify_span_t *out, uint32_t max) {
    if (out == NULL || max == 0) {
        return 0;
    }

    pthread_mutex_lock(&g_trace.lock);
    if (!g_trace.initialized) {
        pthread_mutex_unlock(&g_trace.lock);
        return 0;
    }

    uint32_t available = (uint32_t)(g_trace.write_idx - g_trace.read_idx);
    uint32_t count     = (available < max) ? available : max;

    for (uint32_t i = 0; i < count; i++) {
        uint32_t idx = (uint32_t)(g_trace.read_idx & (uint64_t)(g_trace.cap - 1));
        out[i] = g_trace.ring[idx];
        g_trace.read_idx++;
    }

    pthread_mutex_unlock(&g_trace.lock);
    return count;
}

/* --------------------------------------------------------------------------
 * ify_trace_pending
 * ------------------------------------------------------------------------ */

uint32_t ify_trace_pending(void) {
    pthread_mutex_lock(&g_trace.lock);
    uint32_t n = (uint32_t)(g_trace.write_idx - g_trace.read_idx);
    pthread_mutex_unlock(&g_trace.lock);
    return n;
}
