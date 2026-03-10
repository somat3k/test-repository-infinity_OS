/**
 * @file test_kernel.c
 * @brief Tests for kernel lifecycle, status strings, and version API.
 */

#include "harness.h"
#include <string.h>

#include <infinity/kernel.h>
#include <infinity/ffi.h>

/* Helper: clean slate between tests. */
static void reset(void) {
    ify_kernel_shutdown();
}

static void test_version(void) {
    uint32_t v = ify_kernel_version();
    /* Uninitialized kernel still returns version. */
    TEST_ASSERT(v == INFINITY_KERNEL_VERSION);
    TEST_PASS("version macro matches runtime");
}

static void test_status_str(void) {
    TEST_ASSERT(strcmp(ify_status_str(IFY_OK),                  "ok") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_INVALID_ARG),     "invalid argument") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_OUT_OF_MEMORY),   "out of memory") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_NOT_FOUND),       "not found") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_PERMISSION),      "permission denied") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_OVERFLOW),        "overflow") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_TIMEOUT),         "timeout") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_ALREADY_EXISTS),  "already exists") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_NOT_INITIALIZED), "not initialized") == 0);
    TEST_ASSERT(strcmp(ify_status_str(IFY_ERR_INTERNAL),        "internal error") == 0);
    /* Unknown code should not return NULL. */
    TEST_ASSERT(ify_status_str(-42) != NULL);
    TEST_PASS("status strings");
}

static void test_init_null_opts(void) {
    reset();
    ify_status_t rc = ify_kernel_init(NULL);
    TEST_ASSERT(rc == IFY_ERR_INVALID_ARG);
    TEST_PASS("init with NULL opts returns INVALID_ARG");
}

static void test_init_shutdown(void) {
    reset();
    ify_kernel_opts_t opts;
    __builtin_memset(&opts, 0, sizeof(opts));
    opts.requested_caps = IFY_CAP_MEMORY | IFY_CAP_SCHEDULER;
    opts.max_dimensions = 0;

    ify_status_t rc = ify_kernel_init(&opts);
    TEST_ASSERT(rc == IFY_OK);

    ify_capabilities_t caps = ify_kernel_granted_caps();
    TEST_ASSERT(caps & IFY_CAP_MEMORY);
    TEST_ASSERT(caps & IFY_CAP_SCHEDULER);

    ify_kernel_shutdown();
    TEST_PASS("init → granted caps → shutdown");
}

static void test_double_init(void) {
    reset();
    ify_kernel_opts_t opts;
    __builtin_memset(&opts, 0, sizeof(opts));
    opts.requested_caps = IFY_CAP_MEMORY;

    TEST_ASSERT(ify_kernel_init(&opts) == IFY_OK);
    TEST_ASSERT(ify_kernel_init(&opts) == IFY_ERR_ALREADY_EXISTS);

    ify_kernel_shutdown();
    TEST_PASS("double init returns ALREADY_EXISTS");
}

static void test_shutdown_twice(void) {
    reset();
    ify_kernel_opts_t opts;
    __builtin_memset(&opts, 0, sizeof(opts));
    opts.requested_caps = IFY_CAP_NONE;

    ify_kernel_init(&opts);
    ify_kernel_shutdown();
    /* Second shutdown should be a no-op. */
    ify_kernel_shutdown();
    TEST_PASS("double shutdown is safe");
}

static void test_ffi_before_init(void) {
    reset();
    ify_ffi_abi_t abi;
    __builtin_memset(&abi, 0, sizeof(abi));
    ify_status_t rc = ify_ffi_abi_info(&abi);
    TEST_ASSERT(rc == IFY_ERR_NOT_INITIALIZED);
    TEST_PASS("ffi_abi_info before init returns NOT_INITIALIZED");
}

static void test_ffi_abi_info(void) {
    reset();
    ify_kernel_opts_t opts;
    __builtin_memset(&opts, 0, sizeof(opts));
    opts.requested_caps = IFY_CAP_MEMORY | IFY_CAP_SCHEDULER;
    ify_kernel_init(&opts);

    ify_ffi_abi_t abi;
    __builtin_memset(&abi, 0, sizeof(abi));
    TEST_ASSERT(ify_ffi_abi_info(&abi) == IFY_OK);
    TEST_ASSERT(abi.version == INFINITY_KERNEL_VERSION);
    TEST_ASSERT(abi.struct_size == sizeof(ify_ffi_abi_t));
    TEST_ASSERT(abi.caps_available & IFY_CAP_MEMORY);

    ify_kernel_shutdown();
    TEST_PASS("ffi_abi_info returns correct ABI descriptor");
}

int main(void) {
    test_version();
    test_status_str();
    test_init_null_opts();
    test_init_shutdown();
    test_double_init();
    test_shutdown_twice();
    test_ffi_before_init();
    test_ffi_abi_info();
    printf("All kernel tests passed.\n");
    return 0;
}
