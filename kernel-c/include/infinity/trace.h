/**
 * @file trace.h
 * @brief infinityOS Kernel — Tracing Hooks and Telemetry Interface
 *
 * Provides kernel-level tracing through a span-based model.  Each span
 * records an operation's start and end timestamps (nanoseconds), the
 * owning dimension and task, and an optional parent span for causal trees.
 *
 * Spans are collected in an in-process ring buffer.  An optional emit
 * callback allows external telemetry systems to consume spans as they
 * complete.  Memory and allocation statistics are stamped into spans that
 * originate from the memory subsystem.
 *
 * ABI stability: all structs include a @c _reserved padding field.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_TRACE_H
#define INFINITY_TRACE_H

#include <stdint.h>
#include <stddef.h>
#include "kernel.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * Span record
 * ------------------------------------------------------------------------ */

/** Maximum length (including NUL) of an operation name. */
#define IFY_TRACE_OP_MAX 64

/**
 * @brief A completed execution span.
 *
 * The ring buffer stores spans in this format; the emit callback receives
 * a const pointer to one of these records.
 */
typedef struct {
    uint64_t           span_id;                  /**< Unique monotonic span ID.          */
    uint64_t           parent_span_id;            /**< 0 if this is a root span.          */
    ify_dimension_id_t dimension_id;              /**< Owning dimension (0 if kernel).    */
    ify_task_id_t      task_id;                   /**< Owning task (zeros if kernel).     */
    char               op[IFY_TRACE_OP_MAX];      /**< Operation name.                    */
    uint64_t           start_ns;                  /**< CLOCK_MONOTONIC start, nanoseconds.*/
    uint64_t           end_ns;                    /**< CLOCK_MONOTONIC end, nanoseconds.  */
    /* Memory stats (non-zero when set by memory subsystem spans) */
    uint64_t           alloc_bytes;               /**< Bytes allocated in this span.      */
    uint64_t           free_bytes;                /**< Bytes freed in this span.          */
    uint8_t            _reserved[16];             /**< Reserved; always zero.             */
} ify_span_t;

/* --------------------------------------------------------------------------
 * Emit callback
 * ------------------------------------------------------------------------ */

/**
 * @brief Callback invoked each time a span is completed and stored.
 *
 * The callback is invoked with the ring-buffer lock held; it must not call
 * any ify_trace_* functions to avoid deadlock.
 *
 * @param span  Pointer to the completed span (valid only for the duration
 *              of the callback).
 * @param ctx   Caller-supplied context pointer.
 */
typedef void (*ify_trace_emit_fn_t)(const ify_span_t *span, void *ctx);

/* --------------------------------------------------------------------------
 * Trace API
 * ------------------------------------------------------------------------ */

/**
 * @brief Initialize the trace subsystem with a ring buffer of @p capacity
 *        span slots.
 *
 * Called internally by ify_kernel_init().
 *
 * @param capacity  Number of span slots; must be a power of two and >= 2.
 * @return          IFY_OK on success, or a negative error code.
 */
ify_status_t ify_trace_init(uint32_t capacity);

/**
 * @brief Shut down the trace subsystem and release ring-buffer memory.
 *
 * Called internally by ify_kernel_shutdown().
 */
void ify_trace_shutdown(void);

/**
 * @brief Register an emit callback.
 *
 * Only one callback may be registered at a time.  Passing NULL clears the
 * callback.
 *
 * @param fn   Callback function (or NULL to clear).
 * @param ctx  Context pointer passed to every invocation.
 */
void ify_trace_set_emit(ify_trace_emit_fn_t fn, void *ctx);

/**
 * @brief Begin a new span.
 *
 * @param parent_span_id  Parent span ID for causality; 0 for a root span.
 * @param dimension_id    Owning dimension.
 * @param task_id         Owning task.
 * @param op              NUL-terminated operation name; truncated if longer
 *                        than IFY_TRACE_OP_MAX - 1.
 * @return                Monotonic span ID to pass to ify_trace_end().
 */
uint64_t ify_trace_begin(uint64_t           parent_span_id,
                         ify_dimension_id_t dimension_id,
                         ify_task_id_t      task_id,
                         const char        *op);

/**
 * @brief Complete a span and store it in the ring buffer.
 *
 * @param span_id      Span ID returned by ify_trace_begin().
 * @param alloc_bytes  Bytes allocated during this span (0 if N/A).
 * @param free_bytes   Bytes freed during this span (0 if N/A).
 */
void ify_trace_end(uint64_t span_id,
                   uint64_t alloc_bytes,
                   uint64_t free_bytes);

/**
 * @brief Read up to @p max spans from the ring buffer, oldest first.
 *
 * Spans are consumed (not duplicated); the read pointer advances.
 *
 * @param out  Output array; must hold at least @p max elements.
 * @param max  Maximum number of spans to read.
 * @return     Number of spans actually copied.
 */
uint32_t ify_trace_read(ify_span_t *out, uint32_t max);

/**
 * @brief Return the number of spans currently in the ring buffer.
 */
uint32_t ify_trace_pending(void);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_TRACE_H */
