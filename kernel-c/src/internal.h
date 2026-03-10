/**
 * @file internal.h
 * @brief infinityOS Kernel — Internal shared types and global state.
 *
 * This header is NOT part of the public ABI.  It is used only by kernel
 * source files.  Do not include it from public headers.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_KERNEL_INTERNAL_H
#define INFINITY_KERNEL_INTERNAL_H

#include <stdatomic.h>
#include <pthread.h>
#include <stdint.h>
#include <stddef.h>

#include "../include/infinity/kernel.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * Kernel global state
 * ------------------------------------------------------------------------ */

/** Kernel lifecycle states stored in g_kernel.state. */
#define IFY_KSTATE_UNINIT   0   /**< Not yet initialized.   */
#define IFY_KSTATE_RUNNING  1   /**< Fully operational.     */
#define IFY_KSTATE_SHUTDOWN 2   /**< Shutting down / done.  */

/**
 * @brief Kernel-global singleton — all fields are protected by @c lock
 * except @c state which is modified with compare-exchange.
 */
typedef struct {
    atomic_int              state;            /**< IFY_KSTATE_* value.              */
    pthread_mutex_t         lock;             /**< Protects non-atomic fields.      */
    ify_capabilities_t      granted_caps;     /**< Capabilities granted at init.    */
    uint32_t                max_dimensions;   /**< Hard limit on concurrent dims.   */
    atomic_uint_fast64_t    dim_counter;      /**< Monotonic dimension ID source.   */
} ify_kernel_state_t;

/** Global kernel state instance (defined in kernel.c). */
extern ify_kernel_state_t g_kernel;

/* --------------------------------------------------------------------------
 * Convenience guard macro
 * ------------------------------------------------------------------------ */

/**
 * Return IFY_ERR_NOT_INITIALIZED if the kernel is not in RUNNING state.
 * Use at the top of every public API function that requires an initialized
 * kernel.
 */
#define IFY_REQUIRE_INIT()                                              \
    do {                                                                \
        if (atomic_load_explicit(&g_kernel.state,                       \
                                 memory_order_acquire) != IFY_KSTATE_RUNNING) { \
            return IFY_ERR_NOT_INITIALIZED;                             \
        }                                                               \
    } while (0)

/* --------------------------------------------------------------------------
 * Time helpers (used by scheduler and trace)
 * ------------------------------------------------------------------------ */

/**
 * @brief Return monotonic wall-clock time in nanoseconds.
 *
 * Uses CLOCK_MONOTONIC; falls back to 0 on unsupported platforms.
 */
uint64_t ify_time_now_ns(void);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_KERNEL_INTERNAL_H */
