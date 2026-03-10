/**
 * @file kernel.h
 * @brief infinityOS Kernel — Core Types, Versioning, and Lifecycle API
 *
 * This header is the top-level entry point for the infinityOS C kernel.
 * It defines the foundational types, version macros, and lifecycle functions
 * that all kernel consumers (including the Rust Performer Runtime via FFI) depend on.
 *
 * ABI stability: all symbols exported from this header are stable within a
 * major version.  Breaking changes require a INFINITY_KERNEL_VERSION_MAJOR bump
 * and a corresponding update to ify-ffi in the Rust workspace.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_KERNEL_H
#define INFINITY_KERNEL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * Version
 * ------------------------------------------------------------------------ */

/** Major version — incremented on ABI-breaking changes. */
#define INFINITY_KERNEL_VERSION_MAJOR 0
/** Minor version — incremented on backward-compatible additions. */
#define INFINITY_KERNEL_VERSION_MINOR 1
/** Patch version — incremented on backward-compatible fixes. */
#define INFINITY_KERNEL_VERSION_PATCH 0

/** Packed 32-bit version number: 0xMMmmpppp. */
#define INFINITY_KERNEL_VERSION \
    ((INFINITY_KERNEL_VERSION_MAJOR << 24) | \
     (INFINITY_KERNEL_VERSION_MINOR << 16) | \
     (INFINITY_KERNEL_VERSION_PATCH))

/* --------------------------------------------------------------------------
 * Primitive types
 * ------------------------------------------------------------------------ */

/**
 * @brief Unique task identifier — 128-bit UUID v7 encoded as two 64-bit words.
 *
 * TaskIDs are monotonically increasing within a dimension and must never be
 * reused.  See docs/architecture/taskid-invariants.md for the full invariant
 * specification.
 */
typedef struct {
    uint64_t hi; /**< High 64 bits (timestamp + version + random_a). */
    uint64_t lo; /**< Low  64 bits (variant + random_b).              */
} ify_task_id_t;

/**
 * @brief Unique dimension identifier — 64-bit opaque handle.
 *
 * A dimension is an isolated execution namespace.  See
 * docs/architecture/dimension-model.md for the full dimension-model specification.
 */
typedef uint64_t ify_dimension_id_t;

/**
 * @brief Kernel capability bitmask.
 *
 * Each bit represents a discrete capability that a subsystem may request.
 * Capabilities are granted at kernel boot based on hardware discovery and
 * security policy.  See docs/architecture/capability-registry.md.
 */
typedef uint64_t ify_capabilities_t;

/** No capabilities. */
#define IFY_CAP_NONE            UINT64_C(0)
/** Access to the memory allocator subsystem. */
#define IFY_CAP_MEMORY          (UINT64_C(1) << 0)
/** Access to the scheduler subsystem. */
#define IFY_CAP_SCHEDULER       (UINT64_C(1) << 1)
/** Access to the filesystem namespace (sandboxed). */
#define IFY_CAP_FS              (UINT64_C(1) << 2)
/** Access to the network namespace (sandboxed). */
#define IFY_CAP_NET             (UINT64_C(1) << 3)
/** Access to hardware performance counters. */
#define IFY_CAP_PERF            (UINT64_C(1) << 4)
/** GPU / accelerator access. */
#define IFY_CAP_GPU             (UINT64_C(1) << 5)

/* --------------------------------------------------------------------------
 * Error codes
 * ------------------------------------------------------------------------ */

/** Kernel return code type.  Zero on success, negative on error. */
typedef int32_t ify_status_t;

#define IFY_OK                  ((ify_status_t)  0)
#define IFY_ERR_INVALID_ARG     ((ify_status_t) -1)
#define IFY_ERR_OUT_OF_MEMORY   ((ify_status_t) -2)
#define IFY_ERR_NOT_FOUND       ((ify_status_t) -3)
#define IFY_ERR_PERMISSION      ((ify_status_t) -4)
#define IFY_ERR_OVERFLOW        ((ify_status_t) -5)
#define IFY_ERR_TIMEOUT         ((ify_status_t) -6)
#define IFY_ERR_ALREADY_EXISTS  ((ify_status_t) -7)
#define IFY_ERR_NOT_INITIALIZED ((ify_status_t) -8)
#define IFY_ERR_INTERNAL        ((ify_status_t) -99)

/** Returns a human-readable string for a status code (never NULL). */
const char *ify_status_str(ify_status_t status);

/* --------------------------------------------------------------------------
 * Kernel lifecycle
 * ------------------------------------------------------------------------ */

/**
 * @brief Kernel initialization options passed to ify_kernel_init().
 */
typedef struct {
    /** Requested capability set; kernel may grant a subset. */
    ify_capabilities_t requested_caps;
    /** Maximum number of concurrent dimensions; 0 for default. */
    uint32_t max_dimensions;
    /** Reserved for future use — must be zero-initialized. */
    uint8_t _reserved[48];
} ify_kernel_opts_t;

/**
 * @brief Initialize the kernel subsystems.
 *
 * Must be called exactly once before any other kernel function.  Safe to call
 * from the Rust performer runtime during application startup.
 *
 * @param opts  Initialization options.  Must not be NULL.
 * @return      IFY_OK on success, or a negative error code.
 */
ify_status_t ify_kernel_init(const ify_kernel_opts_t *opts);

/**
 * @brief Shut down the kernel subsystems in a deterministic order.
 *
 * Blocks until all active dimensions and tasks have been drained or timed out.
 * Safe to call from signal handlers and atexit() handlers.
 */
void ify_kernel_shutdown(void);

/**
 * @brief Return the ABI version that this build was compiled with.
 *
 * The Rust ify-ffi crate calls this at startup to verify compatibility.
 *
 * @return Packed version integer (see INFINITY_KERNEL_VERSION).
 */
uint32_t ify_kernel_version(void);

/**
 * @brief Return the capabilities granted to this process at initialization.
 *
 * @return Bitmask of IFY_CAP_* flags.
 */
ify_capabilities_t ify_kernel_granted_caps(void);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_KERNEL_H */
