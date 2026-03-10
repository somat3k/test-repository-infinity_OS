/**
 * @file test_service_registry.c
 * @brief Tests for the kernel service registry and crash-only restart.
 */

#include "harness.h"
#include <string.h>
#include <stdatomic.h>

#include <infinity/kernel.h>
#include <infinity/service_registry.h>

/* --------------------------------------------------------------------------
 * Test service callbacks
 * ------------------------------------------------------------------------ */

static atomic_int g_start_count;
static atomic_int g_stop_count;
static atomic_int g_health_count;
static int        g_start_fail_after; /* Fail for the first N attempts. */

static ify_status_t svc_start(ify_service_id_t id, void *ctx) {
    (void)id; (void)ctx;
    int n = atomic_fetch_add(&g_start_count, 1);
    if (n < g_start_fail_after) {
        return IFY_ERR_INTERNAL;
    }
    return IFY_OK;
}

static void svc_stop(ify_service_id_t id, void *ctx) {
    (void)id; (void)ctx;
    atomic_fetch_add(&g_stop_count, 1);
}

static ify_status_t svc_health(ify_service_id_t id, void *ctx) {
    (void)id; (void)ctx;
    atomic_fetch_add(&g_health_count, 1);
    return IFY_OK;
}

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

static ify_svc_descriptor_t make_desc(const char *name) {
    ify_svc_descriptor_t d;
    memset(&d, 0, sizeof(d));
    strncpy(d.name, name, IFY_SERVICE_NAME_MAX - 1);
    d.start_fn  = svc_start;
    d.stop_fn   = svc_stop;
    d.health_fn = svc_health;
    d.restart_policy.max_restarts    = 0;
    d.restart_policy.backoff_base_ms = 0;
    return d;
}

/* --------------------------------------------------------------------------
 * Tests
 * ------------------------------------------------------------------------ */

static void test_register_start_stop_unregister(void) {
    atomic_store(&g_start_count, 0);
    atomic_store(&g_stop_count,  0);
    g_start_fail_after = 0;
    init_kernel();

    ify_svc_descriptor_t d = make_desc("test-svc");
    ify_service_id_t id = IFY_SERVICE_INVALID;
    TEST_ASSERT(ify_service_register(&d, &id) == IFY_OK);
    TEST_ASSERT(id != IFY_SERVICE_INVALID);

    ify_svc_state_t st;
    TEST_ASSERT(ify_service_state(id, &st) == IFY_OK);
    TEST_ASSERT(st == IFY_SVC_REGISTERED);

    TEST_ASSERT(ify_service_start(id) == IFY_OK);
    TEST_ASSERT(atomic_load(&g_start_count) == 1);

    TEST_ASSERT(ify_service_state(id, &st) == IFY_OK);
    TEST_ASSERT(st == IFY_SVC_RUNNING);

    TEST_ASSERT(ify_service_stop(id) == IFY_OK);
    TEST_ASSERT(atomic_load(&g_stop_count) == 1);

    TEST_ASSERT(ify_service_state(id, &st) == IFY_OK);
    TEST_ASSERT(st == IFY_SVC_STOPPED);

    TEST_ASSERT(ify_service_unregister(id) == IFY_OK);

    ify_kernel_shutdown();
    TEST_PASS("register → start → stop → unregister lifecycle");
}

static void test_duplicate_name(void) {
    init_kernel();

    ify_svc_descriptor_t d = make_desc("dup-svc");
    ify_service_id_t id1 = IFY_SERVICE_INVALID;
    ify_service_id_t id2 = IFY_SERVICE_INVALID;

    TEST_ASSERT(ify_service_register(&d, &id1) == IFY_OK);
    TEST_ASSERT(ify_service_register(&d, &id2) == IFY_ERR_ALREADY_EXISTS);

    ify_kernel_shutdown();
    TEST_PASS("duplicate service name returns ALREADY_EXISTS");
}

static void test_find_by_name(void) {
    init_kernel();

    ify_svc_descriptor_t d = make_desc("findme");
    ify_service_id_t reg_id = IFY_SERVICE_INVALID;
    ify_service_register(&d, &reg_id);

    ify_service_id_t found_id = IFY_SERVICE_INVALID;
    TEST_ASSERT(ify_service_find("findme", &found_id) == IFY_OK);
    TEST_ASSERT(found_id == reg_id);

    TEST_ASSERT(ify_service_find("missing", &found_id) == IFY_ERR_NOT_FOUND);

    ify_kernel_shutdown();
    TEST_PASS("service find by name");
}

static void test_health_check(void) {
    atomic_store(&g_health_count, 0);
    init_kernel();

    ify_svc_descriptor_t d = make_desc("healthy-svc");
    ify_service_id_t id = IFY_SERVICE_INVALID;
    ify_service_register(&d, &id);
    ify_service_start(id);

    TEST_ASSERT(ify_service_health_check(id) == IFY_OK);
    TEST_ASSERT(atomic_load(&g_health_count) == 1);

    ify_kernel_shutdown();
    TEST_PASS("health check invokes callback");
}

static void test_health_check_no_callback(void) {
    init_kernel();

    ify_svc_descriptor_t d = make_desc("no-health");
    d.health_fn = NULL;  /* No health callback. */
    ify_service_id_t id = IFY_SERVICE_INVALID;
    ify_service_register(&d, &id);
    ify_service_start(id);

    /* Should return IFY_OK (assumed healthy). */
    TEST_ASSERT(ify_service_health_check(id) == IFY_OK);

    ify_kernel_shutdown();
    TEST_PASS("health check with NULL callback returns OK");
}

static void test_register_null_desc(void) {
    init_kernel();
    ify_service_id_t id = IFY_SERVICE_INVALID;
    TEST_ASSERT(ify_service_register(NULL, &id) == IFY_ERR_INVALID_ARG);
    ify_kernel_shutdown();
    TEST_PASS("register NULL desc returns INVALID_ARG");
}

static void test_unregister_running_service(void) {
    init_kernel();

    ify_svc_descriptor_t d = make_desc("running-svc");
    ify_service_id_t id = IFY_SERVICE_INVALID;
    ify_service_register(&d, &id);
    ify_service_start(id);

    /* Cannot unregister a running service. */
    TEST_ASSERT(ify_service_unregister(id) == IFY_ERR_INVALID_ARG);

    ify_kernel_shutdown();
    TEST_PASS("unregister running service returns INVALID_ARG");
}

static void test_crash_restart(void) {
    /* Service fails the first time but succeeds on retry. */
    atomic_store(&g_start_count, 0);
    g_start_fail_after = 1; /* Fail first attempt. */
    init_kernel();

    ify_svc_descriptor_t d = make_desc("crashy-svc");
    d.restart_policy.max_restarts    = 2;
    d.restart_policy.backoff_base_ms = 0; /* No delay for test speed. */
    ify_service_id_t id = IFY_SERVICE_INVALID;
    ify_service_register(&d, &id);

    ify_status_t rc = ify_service_start(id);
    TEST_ASSERT(rc == IFY_OK);
    TEST_ASSERT(atomic_load(&g_start_count) == 2); /* Failed once, then succeeded. */

    ify_svc_state_t st;
    ify_service_state(id, &st);
    TEST_ASSERT(st == IFY_SVC_RUNNING);

    g_start_fail_after = 0;
    ify_kernel_shutdown();
    TEST_PASS("crash-only restart: service recovers on retry");
}

static void test_crash_exhaust_restarts(void) {
    /* Service always fails — exhausts restart budget. */
    atomic_store(&g_start_count, 0);
    g_start_fail_after = 100; /* Always fail. */
    init_kernel();

    ify_svc_descriptor_t d = make_desc("always-fail-svc");
    d.restart_policy.max_restarts    = 2;
    d.restart_policy.backoff_base_ms = 0;
    ify_service_id_t id = IFY_SERVICE_INVALID;
    ify_service_register(&d, &id);

    ify_status_t rc = ify_service_start(id);
    TEST_ASSERT(rc != IFY_OK);

    ify_svc_state_t st;
    ify_service_state(id, &st);
    TEST_ASSERT(st == IFY_SVC_FAILED);

    g_start_fail_after = 0;
    ify_kernel_shutdown();
    TEST_PASS("crash-only: service marked FAILED after exhausting retries");
}

int main(void) {
    test_register_start_stop_unregister();
    test_duplicate_name();
    test_find_by_name();
    test_health_check();
    test_health_check_no_callback();
    test_register_null_desc();
    test_unregister_running_service();
    test_crash_restart();
    test_crash_exhaust_restarts();
    printf("All service registry tests passed.\n");
    return 0;
}
