/**
 * @file kernel.c
 * @brief infinityOS Kernel — Core Lifecycle Implementation
 *
 * Implements the boot sequence (init → capability discovery → subsystem
 * start → service loop), shutdown, version query, and status helpers.
 *
 * Boot stages:
 *   1. Pre-init: validate options, compare-exchange state to RUNNING.
 *   2. Capability discovery: intersect requested caps with hardware grants.
 *   3. Subsystem start: initialize service registry and trace subsystem.
 *   4. Ready: state remains RUNNING; kernel is operational.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

/* Needed for clock_gettime / CLOCK_MONOTONIC on POSIX systems. */
#define _POSIX_C_SOURCE 200809L

#include <string.h>
#include <time.h>

#include "internal.h"
#include "../include/infinity/service_registry.h"
#include "../include/infinity/trace.h"

/* --------------------------------------------------------------------------
 * Global kernel state definition
 * ------------------------------------------------------------------------ */

ify_kernel_state_t g_kernel = {
    .state        = ATOMIC_VAR_INIT(IFY_KSTATE_UNINIT),
    .lock         = PTHREAD_MUTEX_INITIALIZER,
    .granted_caps = IFY_CAP_NONE,
    .max_dimensions = 256,
    .dim_counter  = ATOMIC_VAR_INIT(1),
};

/* --------------------------------------------------------------------------
 * Time helper (shared with scheduler and trace)
 * ------------------------------------------------------------------------ */

uint64_t ify_time_now_ns(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return 0;
    }
    return (uint64_t)ts.tv_sec * UINT64_C(1000000000) + (uint64_t)ts.tv_nsec;
}

/* --------------------------------------------------------------------------
 * ify_status_str
 * ------------------------------------------------------------------------ */

const char *ify_status_str(ify_status_t s) {
    switch (s) {
        case IFY_OK:                  return "ok";
        case IFY_ERR_INVALID_ARG:     return "invalid argument";
        case IFY_ERR_OUT_OF_MEMORY:   return "out of memory";
        case IFY_ERR_NOT_FOUND:       return "not found";
        case IFY_ERR_PERMISSION:      return "permission denied";
        case IFY_ERR_OVERFLOW:        return "overflow";
        case IFY_ERR_TIMEOUT:         return "timeout";
        case IFY_ERR_ALREADY_EXISTS:  return "already exists";
        case IFY_ERR_NOT_INITIALIZED: return "not initialized";
        case IFY_ERR_INTERNAL:        return "internal error";
        default:                       return "unknown error";
    }
}

/* --------------------------------------------------------------------------
 * Boot stage helpers
 * ------------------------------------------------------------------------ */

/**
 * Grant a subset of the requested capabilities based on "hardware" discovery.
 * In this implementation all standard capabilities are available.
 */
static ify_capabilities_t discover_capabilities(ify_capabilities_t requested) {
    const ify_capabilities_t available =
        IFY_CAP_MEMORY    |
        IFY_CAP_SCHEDULER |
        IFY_CAP_FS        |
        IFY_CAP_NET       |
        IFY_CAP_PERF      |
        IFY_CAP_GPU;
    return requested & available;
}

/** Start all kernel subsystems in dependency order. */
static ify_status_t start_subsystems(void) {
    ify_status_t rc;

    rc = ify_service_registry_init();
    if (rc != IFY_OK) {
        return rc;
    }

    rc = ify_trace_init(256);
    if (rc != IFY_OK) {
        ify_service_registry_shutdown();
        return rc;
    }

    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_kernel_init
 * ------------------------------------------------------------------------ */

ify_status_t ify_kernel_init(const ify_kernel_opts_t *opts) {
    if (opts == NULL) {
        return IFY_ERR_INVALID_ARG;
    }

    /* Atomically transition UNINIT → RUNNING (prevents double-init). */
    int expected = IFY_KSTATE_UNINIT;
    if (!atomic_compare_exchange_strong_explicit(
            &g_kernel.state, &expected, IFY_KSTATE_RUNNING,
            memory_order_acq_rel, memory_order_acquire)) {
        return IFY_ERR_ALREADY_EXISTS;
    }

    /* Stage 1: capability discovery. */
    g_kernel.granted_caps = discover_capabilities(opts->requested_caps);

    /* Stage 2: apply options. */
    if (opts->max_dimensions != 0) {
        g_kernel.max_dimensions = opts->max_dimensions;
    }

    /* Stage 3: start subsystems. */
    ify_status_t rc = start_subsystems();
    if (rc != IFY_OK) {
        atomic_store_explicit(&g_kernel.state, IFY_KSTATE_UNINIT,
                              memory_order_release);
        return rc;
    }

    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_kernel_shutdown
 * ------------------------------------------------------------------------ */

void ify_kernel_shutdown(void) {
    int expected = IFY_KSTATE_RUNNING;
    if (!atomic_compare_exchange_strong_explicit(
            &g_kernel.state, &expected, IFY_KSTATE_SHUTDOWN,
            memory_order_acq_rel, memory_order_acquire)) {
        return; /* Not running, nothing to do. */
    }

    /* Shut down subsystems in reverse start order. */
    ify_trace_shutdown();
    ify_service_registry_shutdown();

    atomic_store_explicit(&g_kernel.state, IFY_KSTATE_UNINIT,
                          memory_order_release);
}

/* --------------------------------------------------------------------------
 * ify_kernel_version / ify_kernel_granted_caps
 * ------------------------------------------------------------------------ */

uint32_t ify_kernel_version(void) {
    return INFINITY_KERNEL_VERSION;
}

ify_capabilities_t ify_kernel_granted_caps(void) {
    return g_kernel.granted_caps;
}
