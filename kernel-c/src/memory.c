/**
 * @file memory.c
 * @brief infinityOS Kernel — Memory Allocator and Arena Implementation
 *
 * General-purpose allocator: thin wrappers over calloc/realloc/free with
 * bounds-checking contracts enforced in the API layer.
 *
 * Arena allocator: a chain of fixed-size slabs grown on demand.  Each
 * allocation is bump-pointer aligned within the current slab; when the slab
 * is exhausted a new one is appended.  ify_arena_reset() invalidates all
 * pointers but retains the slab chain for reuse.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "internal.h"
#include "../include/infinity/memory.h"

/* --------------------------------------------------------------------------
 * General-purpose allocator
 * ------------------------------------------------------------------------ */

void *ify_malloc(size_t size) {
    if (size == 0) {
        return NULL;
    }
    return calloc(1, size);
}

void *ify_realloc(void *ptr, size_t new_size) {
    if (new_size == 0) {
        free(ptr);
        return NULL;
    }
    return realloc(ptr, new_size);
}

void ify_free(void *ptr) {
    free(ptr);
}

/* --------------------------------------------------------------------------
 * Arena internals
 * ------------------------------------------------------------------------ */

#define IFY_ARENA_DEFAULT_CAP ((size_t)(64u * 1024u))  /* 64 KiB */
#define IFY_ARENA_MAX_ALIGN   ((size_t)4096u)

/** Single backing-store slab. */
typedef struct ify_arena_slab {
    struct ify_arena_slab *next;   /**< Next slab in chain (or NULL). */
    size_t                 cap;    /**< Usable capacity of @c data[]. */
    size_t                 used;   /**< Bytes allocated so far.       */
    /* data follows immediately after this struct in memory */
} ify_arena_slab_t;

/** Return a pointer to the first byte of a slab's data region. */
static char *slab_data(ify_arena_slab_t *s) {
    return (char *)(s + 1);
}

/** Allocate a new slab with at least @p min_cap usable bytes. */
static ify_arena_slab_t *slab_new(size_t min_cap) {
    size_t cap = (min_cap < IFY_ARENA_DEFAULT_CAP) ? IFY_ARENA_DEFAULT_CAP : min_cap;
    /* calloc for zero-init (bounds safety) */
    ify_arena_slab_t *s = (ify_arena_slab_t *)calloc(1, sizeof(*s) + cap);
    if (s == NULL) {
        return NULL;
    }
    s->next = NULL;
    s->cap  = cap;
    s->used = 0;
    return s;
}

/** Free a slab chain starting at @p s. */
static void slab_free_chain(ify_arena_slab_t *s) {
    while (s != NULL) {
        ify_arena_slab_t *next = s->next;
        free(s);
        s = next;
    }
}

struct ify_arena {
    ify_arena_slab_t *head;        /**< First slab (never NULL after create). */
    ify_arena_slab_t *current;     /**< Slab currently being bump-allocated.  */
    uint64_t          alloc_count; /**< Allocations since last reset.         */
};

/* --------------------------------------------------------------------------
 * ify_arena_create
 * ------------------------------------------------------------------------ */

ify_arena_t *ify_arena_create(size_t initial_cap) {
    if (initial_cap == 0) {
        initial_cap = IFY_ARENA_DEFAULT_CAP;
    }
    ify_arena_t *a = (ify_arena_t *)calloc(1, sizeof(*a));
    if (a == NULL) {
        return NULL;
    }
    a->head = slab_new(initial_cap);
    if (a->head == NULL) {
        free(a);
        return NULL;
    }
    a->current     = a->head;
    a->alloc_count = 0;
    return a;
}

/* --------------------------------------------------------------------------
 * ify_arena_alloc
 * ------------------------------------------------------------------------ */

void *ify_arena_alloc(ify_arena_t *arena, size_t size, size_t alignment) {
    if (arena == NULL || size == 0) {
        return NULL;
    }
    if (alignment == 0) {
        alignment = 1;
    }
    /* alignment must be a power of two and <= 4096 */
    if (alignment > IFY_ARENA_MAX_ALIGN || (alignment & (alignment - 1)) != 0) {
        return NULL;
    }

    ify_arena_slab_t *slab = arena->current;

    for (;;) {
        char   *base   = slab_data(slab);
        size_t  offset = slab->used;

        /* Align the actual resulting pointer (base may not be maximally aligned). */
        uintptr_t ptr_val = (uintptr_t)(base + offset);
        uintptr_t aligned = (ptr_val + alignment - 1u) & ~(uintptr_t)(alignment - 1u);
        size_t    pad     = (size_t)(aligned - ptr_val);
        size_t    needed  = pad + size;

        if (offset + needed <= slab->cap) {
            /* Fits in the current slab. */
            slab->used += needed;
            arena->alloc_count++;
            return (void *)aligned;
        }

        /* Doesn't fit — try the next slab or allocate one. */
        if (slab->next == NULL) {
            size_t new_cap = (size > IFY_ARENA_DEFAULT_CAP)
                             ? size + alignment  /* accommodate oversized alloc */
                             : IFY_ARENA_DEFAULT_CAP;
            ify_arena_slab_t *ns = slab_new(new_cap);
            if (ns == NULL) {
                return NULL;
            }
            slab->next = ns;
        }
        slab = slab->next;
        arena->current = slab;
    }
}

/* --------------------------------------------------------------------------
 * ify_arena_reset
 * ------------------------------------------------------------------------ */

void ify_arena_reset(ify_arena_t *arena) {
    if (arena == NULL) {
        return;
    }
    /* Walk and zero all slabs so dangling pointers produce deterministic
     * (zero) values rather than stale data. */
    for (ify_arena_slab_t *s = arena->head; s != NULL; s = s->next) {
        memset(slab_data(s), 0, s->used);
        s->used = 0;
    }
    arena->current     = arena->head;
    arena->alloc_count = 0;
}

/* --------------------------------------------------------------------------
 * ify_arena_destroy
 * ------------------------------------------------------------------------ */

void ify_arena_destroy(ify_arena_t *arena) {
    if (arena == NULL) {
        return;
    }
    slab_free_chain(arena->head);
    free(arena);
}

/* --------------------------------------------------------------------------
 * ify_arena_stats
 * ------------------------------------------------------------------------ */

void ify_arena_stats(const ify_arena_t *arena, ify_arena_stats_t *stats) {
    if (arena == NULL || stats == NULL) {
        return;
    }
    size_t   used     = 0;
    size_t   reserved = 0;
    for (const ify_arena_slab_t *s = arena->head; s != NULL; s = s->next) {
        used     += s->used;
        reserved += s->cap;
    }
    stats->bytes_used     = used;
    stats->bytes_reserved = reserved;
    stats->alloc_count    = arena->alloc_count;
}
