/**
 * @file ffi.h
 * @brief infinityOS Kernel — ABI-Stable FFI Export Surface for Rust
 *
 * This header is the **only** header that the Rust ify-ffi crate may bind to.
 * It re-exports a stable, versioned subset of the kernel API using exclusively
 * C99-compatible types (no VLAs, no compiler extensions, no flexible arrays).
 *
 * Rules for changes to this file:
 * 1. Adding new symbols is backward-compatible (minor version bump).
 * 2. Removing or changing existing symbols requires a major version bump and
 *    a coordinated update to ify-ffi in the Rust workspace.
 * 3. Struct layout changes are forbidden without a major version bump.
 * 4. All new types must include a `_reserved` padding field of at least 8 bytes
 *    for future extensibility.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_FFI_H
#define INFINITY_FFI_H

#include <stddef.h>
#include <stdint.h>

/*
 * Pull in the stable surface from the other kernel headers.
 * Only the types and functions listed here are considered ABI-stable.
 */
#include "kernel.h"
#include "memory.h"
#include "scheduler.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * ABI negotiation — called by ify-ffi at startup
 * ------------------------------------------------------------------------ */

/**
 * @brief ABI descriptor returned by ify_ffi_abi_info().
 *
 * The Rust ify-ffi crate reads this struct to verify that the kernel ABI
 * version matches what it was compiled against.
 */
typedef struct {
    uint32_t version;       /**< INFINITY_KERNEL_VERSION packed integer.    */
    uint32_t struct_size;   /**< sizeof(ify_ffi_abi_t) for layout checks.   */
    uint64_t caps_available;/**< Capabilities available on this hardware.   */
    uint8_t  _reserved[16]; /**< Reserved; must be zero-initialized.        */
} ify_ffi_abi_t;

/**
 * @brief Populate @p out with ABI information.
 *
 * Must be the first FFI call made by the Rust performer runtime after loading
 * the kernel library.  Returns IFY_ERR_NOT_INITIALIZED if ify_kernel_init()
 * has not been called yet.
 *
 * @param out  Output pointer; must not be NULL.
 * @return     IFY_OK on success, or a negative error code.
 */
ify_status_t ify_ffi_abi_info(ify_ffi_abi_t *out);

/* --------------------------------------------------------------------------
 * Dimension management — FFI surface
 * ------------------------------------------------------------------------ */

/**
 * @brief Create a new isolated execution dimension.
 *
 * @param out_id  Output parameter for the new dimension ID; must not be NULL.
 * @return        IFY_OK on success, or a negative error code.
 */
ify_status_t ify_dimension_create(ify_dimension_id_t *out_id);

/**
 * @brief Destroy a dimension and release all associated resources.
 *
 * All schedulers, arenas, and tasks owned by this dimension must be destroyed
 * before calling this function; otherwise IFY_ERR_INVALID_ARG is returned.
 *
 * @param id  Dimension to destroy.
 * @return    IFY_OK on success, or a negative error code.
 */
ify_status_t ify_dimension_destroy(ify_dimension_id_t id);

/* --------------------------------------------------------------------------
 * TaskID generation — FFI surface
 * ------------------------------------------------------------------------ */

/**
 * @brief Generate a new globally-unique TaskID for the given dimension.
 *
 * TaskIDs are UUID v7 (time-ordered) and satisfy the invariants documented in
 * docs/architecture/taskid-invariants.md.
 *
 * @param dimension_id  Owning dimension; used to encode tenancy in the ID.
 * @param out           Output parameter; must not be NULL.
 * @return              IFY_OK on success, or a negative error code.
 */
ify_status_t ify_task_id_generate(ify_dimension_id_t dimension_id, ify_task_id_t *out);

/**
 * @brief Render a TaskID as a 37-character UUID string (including NUL).
 *
 * Format: "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx\0"
 *
 * @param id   TaskID to render.
 * @param buf  Output buffer; must be at least 37 bytes.
 */
void ify_task_id_to_str(ify_task_id_t id, char buf[37]);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_FFI_H */
