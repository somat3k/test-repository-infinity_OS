/**
 * @file test_ffi.c
 * @brief Tests for the FFI export surface: ABI info, dimensions, TaskIDs.
 */

#include "harness.h"
#include <string.h>
#include <stdio.h>

#include <infinity/kernel.h>
#include <infinity/ffi.h>

static void init_kernel(void) {
    ify_kernel_shutdown(); /* Ensure clean state. */
    ify_kernel_opts_t opts;
    memset(&opts, 0, sizeof(opts));
    opts.requested_caps = IFY_CAP_MEMORY | IFY_CAP_SCHEDULER;
    ify_status_t rc = ify_kernel_init(&opts);
    TEST_ASSERT(rc == IFY_OK);
}

/* --------------------------------------------------------------------------
 * ify_ffi_abi_info
 * ------------------------------------------------------------------------ */

static void test_abi_info_null_out(void) {
    init_kernel();
    ify_status_t rc = ify_ffi_abi_info(NULL);
    TEST_ASSERT(rc == IFY_ERR_INVALID_ARG);
    ify_kernel_shutdown();
    TEST_PASS("ffi_abi_info NULL out returns INVALID_ARG");
}

static void test_abi_info_version(void) {
    init_kernel();
    ify_ffi_abi_t abi;
    memset(&abi, 0, sizeof(abi));
    TEST_ASSERT(ify_ffi_abi_info(&abi) == IFY_OK);
    TEST_ASSERT(abi.version == INFINITY_KERNEL_VERSION);
    TEST_ASSERT(abi.struct_size == sizeof(ify_ffi_abi_t));
    ify_kernel_shutdown();
    TEST_PASS("ffi_abi_info version and struct_size");
}

/* --------------------------------------------------------------------------
 * Dimension management
 * ------------------------------------------------------------------------ */

static void test_dimension_create_destroy(void) {
    init_kernel();

    ify_dimension_id_t dim = 0;
    TEST_ASSERT(ify_dimension_create(&dim) == IFY_OK);
    TEST_ASSERT(dim != 0);

    TEST_ASSERT(ify_dimension_destroy(dim) == IFY_OK);

    /* Destroying again should return NOT_FOUND. */
    TEST_ASSERT(ify_dimension_destroy(dim) == IFY_ERR_NOT_FOUND);

    ify_kernel_shutdown();
    TEST_PASS("dimension create/destroy");
}

static void test_dimension_create_null_out(void) {
    init_kernel();
    TEST_ASSERT(ify_dimension_create(NULL) == IFY_ERR_INVALID_ARG);
    ify_kernel_shutdown();
    TEST_PASS("dimension_create NULL out returns INVALID_ARG");
}

static void test_dimension_ids_unique(void) {
    init_kernel();

    ify_dimension_id_t a = 0, b = 0;
    TEST_ASSERT(ify_dimension_create(&a) == IFY_OK);
    TEST_ASSERT(ify_dimension_create(&b) == IFY_OK);
    TEST_ASSERT(a != b);

    ify_dimension_destroy(a);
    ify_dimension_destroy(b);
    ify_kernel_shutdown();
    TEST_PASS("dimension IDs are unique");
}

/* --------------------------------------------------------------------------
 * TaskID generation
 * ------------------------------------------------------------------------ */

static void test_task_id_generate(void) {
    init_kernel();

    ify_dimension_id_t dim = 0;
    ify_dimension_create(&dim);

    ify_task_id_t t1, t2;
    memset(&t1, 0, sizeof(t1));
    memset(&t2, 0, sizeof(t2));

    TEST_ASSERT(ify_task_id_generate(dim, &t1) == IFY_OK);
    TEST_ASSERT(ify_task_id_generate(dim, &t2) == IFY_OK);

    /* IDs must be distinct. */
    TEST_ASSERT(t1.hi != t2.hi || t1.lo != t2.lo);

    ify_dimension_destroy(dim);
    ify_kernel_shutdown();
    TEST_PASS("task IDs are unique within a dimension");
}

static void test_task_id_unknown_dimension(void) {
    init_kernel();

    ify_task_id_t t;
    memset(&t, 0, sizeof(t));
    ify_status_t rc = ify_task_id_generate(0xDEAD, &t);
    TEST_ASSERT(rc == IFY_ERR_NOT_FOUND);

    ify_kernel_shutdown();
    TEST_PASS("task_id_generate on unknown dimension returns NOT_FOUND");
}

/* --------------------------------------------------------------------------
 * ify_task_id_to_str
 * ------------------------------------------------------------------------ */

static void test_task_id_to_str(void) {
    init_kernel();

    ify_dimension_id_t dim = 0;
    ify_dimension_create(&dim);

    ify_task_id_t t;
    memset(&t, 0, sizeof(t));
    ify_task_id_generate(dim, &t);

    char buf[37];
    memset(buf, 0, sizeof(buf));
    ify_task_id_to_str(t, buf);

    /* Should be exactly 36 characters + NUL. */
    size_t len = 0;
    while (buf[len] != '\0') len++;
    TEST_ASSERT(len == 36);

    /* Format: 8-4-4-4-12 hex digits with hyphens. */
    TEST_ASSERT(buf[8]  == '-');
    TEST_ASSERT(buf[13] == '-');
    TEST_ASSERT(buf[18] == '-');
    TEST_ASSERT(buf[23] == '-');

    ify_dimension_destroy(dim);
    ify_kernel_shutdown();
    TEST_PASS("task_id_to_str produces valid UUID string");
}

int main(void) {
    test_abi_info_null_out();
    test_abi_info_version();
    test_dimension_create_destroy();
    test_dimension_create_null_out();
    test_dimension_ids_unique();
    test_task_id_generate();
    test_task_id_unknown_dimension();
    test_task_id_to_str();
    printf("All FFI tests passed.\n");
    return 0;
}
