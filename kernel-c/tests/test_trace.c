/**
 * @file test_trace.c
 * @brief Tests for the kernel tracing / telemetry subsystem.
 */

#include "harness.h"
#include <string.h>
#include <stdatomic.h>

#include <infinity/kernel.h>
#include <infinity/trace.h>

/* --------------------------------------------------------------------------
 * Helpers
 * ------------------------------------------------------------------------ */

static void init_kernel(void) {
    ify_kernel_shutdown();
    ify_kernel_opts_t opts;
    memset(&opts, 0, sizeof(opts));
    opts.requested_caps = IFY_CAP_MEMORY;
    TEST_ASSERT(ify_kernel_init(&opts) == IFY_OK);
}

static atomic_int g_emit_count;

static void emit_cb(const ify_span_t *span, void *ctx) {
    (void)span; (void)ctx;
    atomic_fetch_add(&g_emit_count, 1);
}

/* --------------------------------------------------------------------------
 * Tests
 * ------------------------------------------------------------------------ */

static void test_begin_end_read(void) {
    init_kernel();

    uint32_t before = ify_trace_pending();
    TEST_ASSERT(before == 0);

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));

    uint64_t sid = ify_trace_begin(0, 1, tid, "test.op");
    TEST_ASSERT(sid != 0);

    ify_trace_end(sid, 100, 0);

    TEST_ASSERT(ify_trace_pending() == 1);

    ify_span_t spans[4];
    uint32_t n = ify_trace_read(spans, 4);
    TEST_ASSERT(n == 1);
    TEST_ASSERT(spans[0].span_id == sid);
    TEST_ASSERT(strcmp(spans[0].op, "test.op") == 0);
    TEST_ASSERT(spans[0].alloc_bytes == 100);
    TEST_ASSERT(spans[0].end_ns >= spans[0].start_ns);

    TEST_ASSERT(ify_trace_pending() == 0);

    ify_kernel_shutdown();
    TEST_PASS("trace begin/end/read basic");
}

static void test_parent_span(void) {
    init_kernel();

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));

    uint64_t parent = ify_trace_begin(0, 1, tid, "parent");
    uint64_t child  = ify_trace_begin(parent, 1, tid, "child");

    ify_trace_end(child, 0, 0);
    ify_trace_end(parent, 0, 0);

    ify_span_t spans[4];
    uint32_t n = ify_trace_read(spans, 4);
    TEST_ASSERT(n == 2);

    /* Child was ended first so it should appear first in ring. */
    TEST_ASSERT(spans[0].span_id == child);
    TEST_ASSERT(spans[0].parent_span_id == parent);
    TEST_ASSERT(spans[1].span_id == parent);
    TEST_ASSERT(spans[1].parent_span_id == 0);

    ify_kernel_shutdown();
    TEST_PASS("parent/child span causality");
}

static void test_emit_callback(void) {
    atomic_store(&g_emit_count, 0);
    init_kernel();

    ify_trace_set_emit(emit_cb, NULL);

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    uint64_t sid = ify_trace_begin(0, 0, tid, "emit-test");
    ify_trace_end(sid, 0, 0);

    TEST_ASSERT(atomic_load(&g_emit_count) == 1);

    ify_trace_set_emit(NULL, NULL);
    ify_kernel_shutdown();
    TEST_PASS("emit callback fires on span completion");
}

static void test_op_truncation(void) {
    init_kernel();

    /* Operation name longer than IFY_TRACE_OP_MAX - 1 must be truncated. */
    char long_op[IFY_TRACE_OP_MAX + 32];
    memset(long_op, 'x', sizeof(long_op) - 1);
    long_op[sizeof(long_op) - 1] = '\0';

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    uint64_t sid = ify_trace_begin(0, 0, tid, long_op);
    ify_trace_end(sid, 0, 0);

    ify_span_t span;
    ify_trace_read(&span, 1);
    /* NUL terminator must be within the op buffer. */
    TEST_ASSERT(span.op[IFY_TRACE_OP_MAX - 1] == '\0');

    ify_kernel_shutdown();
    TEST_PASS("operation name is truncated safely");
}

static void test_ring_overflow(void) {
    init_kernel();

    /* Write more spans than the ring capacity (256) — oldest should be lost. */
    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    for (int i = 0; i < 300; i++) {
        uint64_t s = ify_trace_begin(0, 0, tid, "overflow");
        ify_trace_end(s, 0, 0);
    }
    /* Ring holds at most 256 spans. */
    uint32_t pending = ify_trace_pending();
    TEST_ASSERT(pending <= 256);

    /* Drain. */
    ify_span_t buf[256];
    ify_trace_read(buf, 256);

    ify_kernel_shutdown();
    TEST_PASS("ring buffer handles overflow without crash");
}

static void test_end_unknown_span(void) {
    init_kernel();
    /* Ending a span_id that was never begun should be a silent no-op. */
    ify_trace_end(0xDEADBEEF, 0, 0);
    TEST_ASSERT(ify_trace_pending() == 0);
    ify_kernel_shutdown();
    TEST_PASS("end unknown span_id is a no-op");
}

static void test_read_empty(void) {
    init_kernel();
    ify_span_t buf[4];
    uint32_t n = ify_trace_read(buf, 4);
    TEST_ASSERT(n == 0);
    ify_kernel_shutdown();
    TEST_PASS("read from empty ring returns 0");
}

int main(void) {
    test_begin_end_read();
    test_parent_span();
    test_emit_callback();
    test_op_truncation();
    test_ring_overflow();
    test_end_unknown_span();
    test_read_empty();
    printf("All trace tests passed.\n");
    return 0;
}
