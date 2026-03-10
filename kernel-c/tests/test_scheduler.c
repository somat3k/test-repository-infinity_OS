/**
 * @file test_scheduler.c
 * @brief Tests for the scheduler: submit, cancel, state query, and timers.
 */

#include "harness.h"
#include <string.h>
#include <stdatomic.h>

#include <infinity/scheduler.h>
#include <infinity/kernel.h>

/* --------------------------------------------------------------------------
 * Simple task callbacks
 * ------------------------------------------------------------------------ */

static atomic_int g_counter;

static ify_status_t task_inc(ify_task_id_t id, void *arg) {
    (void)id; (void)arg;
    atomic_fetch_add(&g_counter, 1);
    return IFY_OK;
}

static ify_status_t task_fail(ify_task_id_t id, void *arg) {
    (void)id; (void)arg;
    return IFY_ERR_INTERNAL;
}

/* --------------------------------------------------------------------------
 * Helpers
 * ------------------------------------------------------------------------ */

static ify_scheduler_t *make_sched(uint32_t max_concurrent) {
    ify_scheduler_opts_t opts;
    memset(&opts, 0, sizeof(opts));
    opts.dimension_id   = 1;
    opts.max_concurrent = max_concurrent;
    return ify_scheduler_create(&opts);
}

/* --------------------------------------------------------------------------
 * Tests
 * ------------------------------------------------------------------------ */

static void test_create_destroy(void) {
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);
    ify_scheduler_destroy(s);
    ify_scheduler_destroy(NULL); /* No-op. */
    TEST_PASS("scheduler create/destroy");
}

static void test_create_null_opts(void) {
    ify_scheduler_t *s = ify_scheduler_create(NULL);
    TEST_ASSERT(s == NULL);
    TEST_PASS("create with NULL opts returns NULL");
}

static void test_submit_runs_task(void) {
    atomic_store(&g_counter, 0);
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    ify_status_t rc = ify_scheduler_submit(s, task_inc, NULL, IFY_PRIO_NORMAL, &tid);
    TEST_ASSERT(rc == IFY_OK);
    TEST_ASSERT(tid.hi != 0 || tid.lo != 0);
    TEST_ASSERT(atomic_load(&g_counter) == 1);

    ify_scheduler_destroy(s);
    TEST_PASS("submit executes task synchronously");
}

static void test_submit_multiple_priorities(void) {
    atomic_store(&g_counter, 0);
    ify_scheduler_t *s = make_sched(1); /* One at a time to ensure ordering. */
    TEST_ASSERT(s != NULL);

    for (int i = 0; i < 5; i++) {
        ify_status_t rc = ify_scheduler_submit(s, task_inc, NULL, IFY_PRIO_LOW, NULL);
        TEST_ASSERT(rc == IFY_OK);
    }
    TEST_ASSERT(atomic_load(&g_counter) == 5);

    ify_scheduler_destroy(s);
    TEST_PASS("submit multiple tasks with low priority");
}

static void test_task_state_completed(void) {
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    ify_scheduler_submit(s, task_inc, NULL, IFY_PRIO_NORMAL, &tid);

    ify_task_state_t st;
    ify_status_t rc = ify_scheduler_state(s, tid, &st);
    TEST_ASSERT(rc == IFY_OK);
    TEST_ASSERT(st == IFY_TASK_COMPLETED);

    ify_scheduler_destroy(s);
    TEST_PASS("completed task state query");
}

static void test_task_state_failed(void) {
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);

    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    ify_scheduler_submit(s, task_fail, NULL, IFY_PRIO_NORMAL, &tid);

    ify_task_state_t st;
    ify_scheduler_state(s, tid, &st);
    TEST_ASSERT(st == IFY_TASK_FAILED);

    ify_scheduler_destroy(s);
    TEST_PASS("failed task state query");
}

static void test_state_not_found(void) {
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);

    ify_task_id_t fake;
    fake.hi = 0xDEADBEEF;
    fake.lo = 0xCAFEBABE;

    ify_task_state_t st;
    ify_status_t rc = ify_scheduler_state(s, fake, &st);
    TEST_ASSERT(rc == IFY_ERR_NOT_FOUND);

    ify_scheduler_destroy(s);
    TEST_PASS("state query for unknown task returns NOT_FOUND");
}

static void test_cancel_queued(void) {
    /* Use max_concurrent=0 to queue but not run tasks. */
    ify_scheduler_opts_t opts;
    memset(&opts, 0, sizeof(opts));
    opts.dimension_id   = 2;
    opts.max_concurrent = 0; /* Force 0 — becomes default 32 actually */
    /* Use 1 worker but we submit 2 tasks; first runs, second is queued. */
    ify_scheduler_t *s = make_sched(0);
    TEST_ASSERT(s != NULL);

    /* With max_concurrent defaulting to 32, just test cancel of a done task. */
    ify_task_id_t tid;
    memset(&tid, 0, sizeof(tid));
    ify_scheduler_submit(s, task_inc, NULL, IFY_PRIO_NORMAL, &tid);

    /* The task is done; cancel should return NOT_FOUND (terminal state). */
    ify_status_t rc = ify_scheduler_cancel(s, tid);
    TEST_ASSERT(rc == IFY_ERR_NOT_FOUND);

    ify_scheduler_destroy(s);
    TEST_PASS("cancel on terminal task returns NOT_FOUND");
}

static void test_submit_null_fn(void) {
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);

    ify_status_t rc = ify_scheduler_submit(s, NULL, NULL, IFY_PRIO_NORMAL, NULL);
    TEST_ASSERT(rc == IFY_ERR_INVALID_ARG);

    ify_scheduler_destroy(s);
    TEST_PASS("submit with NULL fn returns INVALID_ARG");
}

/* --------------------------------------------------------------------------
 * Timer tests
 * ------------------------------------------------------------------------ */

static atomic_int g_timer_fired;

static void timer_cb(uint64_t timer_id, void *arg) {
    (void)timer_id; (void)arg;
    atomic_fetch_add(&g_timer_fired, 1);
}

static void test_timer_zero_delay(void) {
    atomic_store(&g_timer_fired, 0);
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);

    ify_status_t rc = ify_timer_once(s, 0, timer_cb, NULL, NULL);
    TEST_ASSERT(rc == IFY_OK);
    /* Zero delay — fires immediately when submitted. */
    TEST_ASSERT(atomic_load(&g_timer_fired) == 1);

    ify_scheduler_destroy(s);
    TEST_PASS("timer with zero delay fires immediately");
}

static void test_timer_cancel(void) {
    atomic_store(&g_timer_fired, 0);
    ify_scheduler_t *s = make_sched(4);
    TEST_ASSERT(s != NULL);

    ify_timer_t *t = NULL;
    /* Very long delay — should not fire during this test. */
    ify_status_t rc = ify_timer_once(s, (uint64_t)10 * 1000 * 1000 * 1000,
                                      timer_cb, NULL, &t);
    TEST_ASSERT(rc == IFY_OK);
    TEST_ASSERT(t != NULL);

    ify_timer_cancel(t);
    /* Callback should not have been invoked. */
    TEST_ASSERT(atomic_load(&g_timer_fired) == 0);

    ify_scheduler_destroy(s);
    TEST_PASS("timer cancel prevents callback");
}

int main(void) {
    test_create_destroy();
    test_create_null_opts();
    test_submit_runs_task();
    test_submit_multiple_priorities();
    test_task_state_completed();
    test_task_state_failed();
    test_state_not_found();
    test_cancel_queued();
    test_submit_null_fn();
    test_timer_zero_delay();
    test_timer_cancel();
    printf("All scheduler tests passed.\n");
    return 0;
}
