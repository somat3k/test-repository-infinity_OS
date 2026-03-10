/**
 * @file test_replication.c
 * @brief Tests for the replication kernel.
 */

#include "harness.h"
#include <string.h>
#include <stdatomic.h>

#include <infinity/kernel.h>
#include <infinity/replication.h>
#include <infinity/memory.h>
#include <infinity/scheduler.h>

/* --------------------------------------------------------------------------
 * Helpers
 * ------------------------------------------------------------------------ */

static void init_kernel(void) {
    ify_kernel_shutdown();
    ify_kernel_opts_t opts;
    memset(&opts, 0, sizeof(opts));
    opts.requested_caps = IFY_CAP_MEMORY | IFY_CAP_SCHEDULER;
    TEST_ASSERT(ify_kernel_init(&opts) == IFY_OK);
}

static ify_replica_opts_t make_opts(void) {
    ify_replica_opts_t o;
    memset(&o, 0, sizeof(o));
    o.dimension_id          = 42;
    o.policy.max_tasks      = 8;
    o.policy.arena_cap_bytes = 64 * 1024;
    o.policy.auto_destroy   = 1;
    o.policy.cpu_pin        = UINT8_MAX;
    return o;
}

static atomic_int g_task_counter;

static ify_status_t replica_task(ify_task_id_t id, void *arg) {
    (void)id; (void)arg;
    atomic_fetch_add(&g_task_counter, 1);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * Tests
 * ------------------------------------------------------------------------ */

static void test_create_destroy(void) {
    init_kernel();

    ify_replica_opts_t opts = make_opts();
    ify_replica_id_t id = IFY_REPLICA_INVALID;
    TEST_ASSERT(ify_replica_create(&opts, &id) == IFY_OK);
    TEST_ASSERT(id != IFY_REPLICA_INVALID);

    ify_replica_state_t st;
    TEST_ASSERT(ify_replica_state(id, &st) == IFY_OK);
    TEST_ASSERT(st == IFY_REPLICA_CREATED);

    TEST_ASSERT(ify_replica_destroy(id) == IFY_OK);

    /* Destroy again returns NOT_FOUND. */
    TEST_ASSERT(ify_replica_destroy(id) == IFY_ERR_NOT_FOUND);

    /* Destroying IFY_REPLICA_INVALID is a no-op. */
    TEST_ASSERT(ify_replica_destroy(IFY_REPLICA_INVALID) == IFY_OK);

    ify_kernel_shutdown();
    TEST_PASS("replica create/destroy lifecycle");
}

static void test_create_null_opts(void) {
    init_kernel();
    ify_replica_id_t id = IFY_REPLICA_INVALID;
    TEST_ASSERT(ify_replica_create(NULL, &id) == IFY_ERR_INVALID_ARG);
    ify_kernel_shutdown();
    TEST_PASS("replica_create with NULL opts returns INVALID_ARG");
}

static void test_create_null_out(void) {
    init_kernel();
    ify_replica_opts_t opts = make_opts();
    TEST_ASSERT(ify_replica_create(&opts, NULL) == IFY_ERR_INVALID_ARG);
    ify_kernel_shutdown();
    TEST_PASS("replica_create with NULL out_id returns INVALID_ARG");
}

static void test_submit_task(void) {
    atomic_store(&g_task_counter, 0);
    init_kernel();

    ify_replica_opts_t opts = make_opts();
    ify_replica_id_t id = IFY_REPLICA_INVALID;
    ify_replica_create(&opts, &id);

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    TEST_ASSERT(ify_replica_submit(id, replica_task, NULL, IFY_PRIO_NORMAL, &tid) == IFY_OK);
    TEST_ASSERT(atomic_load(&g_task_counter) == 1);

    ify_replica_destroy(id);
    ify_kernel_shutdown();
    TEST_PASS("replica submit executes task");
}

static void test_arena_accessible(void) {
    init_kernel();

    ify_replica_opts_t opts = make_opts();
    ify_replica_id_t id = IFY_REPLICA_INVALID;
    ify_replica_create(&opts, &id);

    ify_arena_t *a = ify_replica_arena(id);
    TEST_ASSERT(a != NULL);

    void *p = ify_arena_alloc(a, 128, 8);
    TEST_ASSERT(p != NULL);

    ify_replica_destroy(id);
    ify_kernel_shutdown();
    TEST_PASS("replica arena is accessible and functional");
}

static void test_scheduler_accessible(void) {
    init_kernel();

    ify_replica_opts_t opts = make_opts();
    ify_replica_id_t id = IFY_REPLICA_INVALID;
    ify_replica_create(&opts, &id);

    ify_scheduler_t *s = ify_replica_scheduler(id);
    TEST_ASSERT(s != NULL);

    ify_replica_destroy(id);
    ify_kernel_shutdown();
    TEST_PASS("replica scheduler is accessible");
}

static void test_ids_unique(void) {
    init_kernel();

    ify_replica_opts_t opts = make_opts();
    ify_replica_id_t a = IFY_REPLICA_INVALID, b = IFY_REPLICA_INVALID;
    ify_replica_create(&opts, &a);
    ify_replica_create(&opts, &b);
    TEST_ASSERT(a != b);

    ify_replica_destroy(a);
    ify_replica_destroy(b);
    ify_kernel_shutdown();
    TEST_PASS("replica IDs are unique");
}

static void test_submit_invalid(void) {
    init_kernel();
    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    TEST_ASSERT(ify_replica_submit(IFY_REPLICA_INVALID, replica_task, NULL, IFY_PRIO_NORMAL, &tid) == IFY_ERR_INVALID_ARG);
    TEST_ASSERT(ify_replica_submit(99999u, replica_task, NULL, IFY_PRIO_NORMAL, &tid) == IFY_ERR_NOT_FOUND);
    ify_kernel_shutdown();
    TEST_PASS("submit to invalid/missing replica returns error");
}

static void test_state_not_found(void) {
    init_kernel();
    ify_replica_state_t st;
    TEST_ASSERT(ify_replica_state(99999u, &st) == IFY_ERR_NOT_FOUND);
    ify_kernel_shutdown();
    TEST_PASS("state query for unknown replica returns NOT_FOUND");
}

int main(void) {
    test_create_destroy();
    test_create_null_opts();
    test_create_null_out();
    test_submit_task();
    test_arena_accessible();
    test_scheduler_accessible();
    test_ids_unique();
    test_submit_invalid();
    test_state_not_found();
    printf("All replication tests passed.\n");
    return 0;
}
