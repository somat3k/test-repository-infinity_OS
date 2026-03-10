/**
 * @file memory.h
 * @brief infinityOS Kernel — Memory Allocator and Arena Interface
 *
 * Provides the kernel-level memory primitives consumed by all other kernel
 * subsystems and re-exported to the Rust Performer Runtime via ffi.h.
 *
 * Design principles:
 * - Arena allocation for short-lived, dimension-scoped allocations.
 * - General-purpose allocator with refcount semantics for longer-lived objects.
 * - Strict bounds checking at every allocation boundary.
 * - Deterministic tear-down: arenas and their allocations are freed together.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_MEMORY_H
#define INFINITY_MEMORY_H

#include <stddef.h>
#include <stdint.h>
#include "kernel.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * General-purpose allocator
 * ------------------------------------------------------------------------ */

/**
 * @brief Allocate @p size bytes of zero-initialized memory.
 *
 * Equivalent to calloc(1, size) but returns NULL on size == 0 and always
 * zero-fills on success.
 *
 * @param size  Number of bytes to allocate.
 * @return      Pointer to allocated memory, or NULL on failure.
 */
void *ify_malloc(size_t size);

/**
 * @brief Resize a previous allocation.
 *
 * On growth the new bytes are zero-filled.  On failure the original pointer
 * is left unchanged and NULL is returned.
 *
 * @param ptr       Pointer returned by a previous ify_malloc / ify_realloc.
 * @param new_size  Requested new size in bytes.
 * @return          Pointer to resized memory, or NULL on failure.
 */
void *ify_realloc(void *ptr, size_t new_size);

/**
 * @brief Free memory previously allocated by ify_malloc / ify_realloc.
 *
 * Passing NULL is a no-op.
 *
 * @param ptr  Pointer to free.
 */
void ify_free(void *ptr);

/* --------------------------------------------------------------------------
 * Arena allocator
 * ------------------------------------------------------------------------ */

/**
 * @brief Opaque arena handle.
 *
 * Arenas provide O(1) bump allocation and O(1) bulk free.  All allocations
 * within an arena are freed together when ify_arena_destroy() is called.
 * Arenas are not thread-safe; use one arena per dimension or protect with an
 * external lock.
 */
typedef struct ify_arena ify_arena_t;

/**
 * @brief Create a new arena with an initial backing-store capacity.
 *
 * The arena will grow automatically as needed.
 *
 * @param initial_cap  Initial capacity in bytes; 0 selects a default (64 KiB).
 * @return             Pointer to the new arena, or NULL on allocation failure.
 */
ify_arena_t *ify_arena_create(size_t initial_cap);

/**
 * @brief Allocate @p size bytes from the arena, aligned to @p alignment.
 *
 * @param arena      Arena handle; must not be NULL.
 * @param size       Number of bytes to allocate.
 * @param alignment  Required alignment; must be a power of two and <= 4096.
 * @return           Pointer to allocated memory, or NULL on failure.
 */
void *ify_arena_alloc(ify_arena_t *arena, size_t size, size_t alignment);

/**
 * @brief Reset an arena without freeing its backing store.
 *
 * All previous allocations are invalidated.  The backing store is retained
 * for reuse, avoiding repeated OS allocations for recurring workloads.
 *
 * @param arena  Arena handle; must not be NULL.
 */
void ify_arena_reset(ify_arena_t *arena);

/**
 * @brief Destroy an arena and release its backing store.
 *
 * Passing NULL is a no-op.  All pointers into the arena become invalid.
 *
 * @param arena  Arena handle to destroy.
 */
void ify_arena_destroy(ify_arena_t *arena);

/**
 * @brief Return current allocation statistics for an arena.
 */
typedef struct {
    size_t bytes_used;      /**< Bytes currently in use.           */
    size_t bytes_reserved;  /**< Total bytes in backing store(s).  */
    uint64_t alloc_count;   /**< Number of allocations since last reset. */
} ify_arena_stats_t;

/**
 * @brief Populate @p stats with current statistics for @p arena.
 *
 * @param arena  Arena handle; must not be NULL.
 * @param stats  Output parameter; must not be NULL.
 */
void ify_arena_stats(const ify_arena_t *arena, ify_arena_stats_t *stats);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_MEMORY_H */
