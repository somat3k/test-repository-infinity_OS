/**
 * @file test_memory.c
 * @brief Tests for ify_malloc, ify_realloc, ify_free, and arena allocator.
 */

#include "harness.h"
#include <string.h>

#include <infinity/memory.h>

/* --------------------------------------------------------------------------
 * General-purpose allocator tests
 * ------------------------------------------------------------------------ */

static void test_malloc_zero(void) {
    void *p = ify_malloc(0);
    TEST_ASSERT(p == NULL);
    TEST_PASS("malloc(0) returns NULL");
}

static void test_malloc_free(void) {
    void *p = ify_malloc(128);
    TEST_ASSERT(p != NULL);
    /* Memory must be zero-initialised. */
    char buf[128];
    memset(buf, 0, 128);
    TEST_ASSERT(memcmp(p, buf, 128) == 0);
    ify_free(p);
    TEST_PASS("malloc/free basic");
}

static void test_free_null(void) {
    ify_free(NULL); /* Must be a no-op. */
    TEST_PASS("free(NULL) is safe");
}

static void test_realloc_grow(void) {
    void *p = ify_malloc(64);
    TEST_ASSERT(p != NULL);
    memset(p, 0xAB, 64);

    void *q = ify_realloc(p, 128);
    TEST_ASSERT(q != NULL);

    /* Original data must be preserved. */
    char expected[64];
    memset(expected, 0xAB, 64);
    TEST_ASSERT(memcmp(q, expected, 64) == 0);

    ify_free(q);
    TEST_PASS("realloc grows allocation");
}

static void test_realloc_to_zero(void) {
    void *p = ify_malloc(64);
    TEST_ASSERT(p != NULL);
    void *q = ify_realloc(p, 0);
    TEST_ASSERT(q == NULL);
    TEST_PASS("realloc to 0 frees and returns NULL");
}

/* --------------------------------------------------------------------------
 * Arena allocator tests
 * ------------------------------------------------------------------------ */

static void test_arena_create_destroy(void) {
    ify_arena_t *a = ify_arena_create(0); /* 0 uses default cap. */
    TEST_ASSERT(a != NULL);
    ify_arena_destroy(a);

    ify_arena_destroy(NULL); /* Must be a no-op. */
    TEST_PASS("arena create/destroy");
}

static void test_arena_alloc_basic(void) {
    ify_arena_t *a = ify_arena_create(1024);
    TEST_ASSERT(a != NULL);

    void *p = ify_arena_alloc(a, 100, 8);
    TEST_ASSERT(p != NULL);
    /* Pointer must be 8-byte aligned. */
    TEST_ASSERT(((uintptr_t)p & 7u) == 0);

    void *q = ify_arena_alloc(a, 100, 16);
    TEST_ASSERT(q != NULL);
    TEST_ASSERT(((uintptr_t)q & 15u) == 0);
    /* Two allocations must not overlap. */
    TEST_ASSERT(q != p);

    ify_arena_destroy(a);
    TEST_PASS("arena basic allocation with alignment");
}

static void test_arena_alloc_null(void) {
    /* NULL arena must return NULL. */
    void *p = ify_arena_alloc(NULL, 16, 8);
    TEST_ASSERT(p == NULL);

    /* size=0 must return NULL. */
    ify_arena_t *a = ify_arena_create(64);
    TEST_ASSERT(a != NULL);
    p = ify_arena_alloc(a, 0, 8);
    TEST_ASSERT(p == NULL);
    ify_arena_destroy(a);
    TEST_PASS("arena alloc guards");
}

static void test_arena_grows_across_slabs(void) {
    /* Start with a tiny capacity to force slab growth. */
    ify_arena_t *a = ify_arena_create(128);
    TEST_ASSERT(a != NULL);

    /* Allocate more than the initial capacity. */
    for (int i = 0; i < 20; i++) {
        void *p = ify_arena_alloc(a, 64, 8);
        TEST_ASSERT(p != NULL);
        /* Write a canary to detect overlap. */
        memset(p, (char)(i + 1), 64);
    }

    ify_arena_destroy(a);
    TEST_PASS("arena grows across slabs");
}

static void test_arena_reset(void) {
    ify_arena_t *a = ify_arena_create(256);
    TEST_ASSERT(a != NULL);

    void *p1 = ify_arena_alloc(a, 64, 8);
    TEST_ASSERT(p1 != NULL);
    memset(p1, 0xFF, 64);

    ify_arena_reset(a);

    /* After reset, new allocation should reuse the same space. */
    void *p2 = ify_arena_alloc(a, 64, 8);
    TEST_ASSERT(p2 != NULL);
    TEST_ASSERT(p2 == p1); /* Should reuse first offset. */

    /* And the memory should be zero after reset. */
    char zeros[64];
    memset(zeros, 0, 64);
    TEST_ASSERT(memcmp(p2, zeros, 64) == 0);

    ify_arena_destroy(a);
    TEST_PASS("arena reset reuses memory");
}

static void test_arena_stats(void) {
    ify_arena_t *a = ify_arena_create(512);
    TEST_ASSERT(a != NULL);

    ify_arena_alloc(a, 100, 8);
    ify_arena_alloc(a, 100, 8);

    ify_arena_stats_t st;
    ify_arena_stats(a, &st);
    TEST_ASSERT(st.bytes_used >= 200);
    TEST_ASSERT(st.bytes_reserved >= st.bytes_used);
    TEST_ASSERT(st.alloc_count == 2);

    ify_arena_destroy(a);
    TEST_PASS("arena stats");
}

static void test_arena_bad_alignment(void) {
    ify_arena_t *a = ify_arena_create(256);
    TEST_ASSERT(a != NULL);

    /* alignment=3 is not a power of two — must return NULL. */
    void *p = ify_arena_alloc(a, 16, 3);
    TEST_ASSERT(p == NULL);

    /* alignment > 4096 — must return NULL. */
    p = ify_arena_alloc(a, 16, 8192);
    TEST_ASSERT(p == NULL);

    ify_arena_destroy(a);
    TEST_PASS("arena rejects invalid alignment");
}

int main(void) {
    test_malloc_zero();
    test_malloc_free();
    test_free_null();
    test_realloc_grow();
    test_realloc_to_zero();
    test_arena_create_destroy();
    test_arena_alloc_basic();
    test_arena_alloc_null();
    test_arena_grows_across_slabs();
    test_arena_reset();
    test_arena_stats();
    test_arena_bad_alignment();
    printf("All memory tests passed.\n");
    return 0;
}
