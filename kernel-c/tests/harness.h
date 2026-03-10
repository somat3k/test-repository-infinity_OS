/**
 * @file harness.h
 * @brief Simple assert-based test harness for kernel-c tests.
 *
 * No external dependencies — uses only <stdio.h>, <stdlib.h>.
 *
 * Usage:
 *   - Call TEST_ASSERT(cond) to check a condition.
 *   - Call TEST_PASS(name) to print a pass message (optional).
 *   - Return 0 from main() to signal overall success.
 */

#ifndef INFINITY_TEST_HARNESS_H
#define INFINITY_TEST_HARNESS_H

#include <stdio.h>
#include <stdlib.h>

/** Abort the test with a message if @p cond is false. */
#define TEST_ASSERT(cond) \
    do { \
        if (!(cond)) { \
            fprintf(stderr, "FAIL  %s:%d  %s\n", __FILE__, __LINE__, #cond); \
            exit(1); \
        } \
    } while (0)

/** Print a PASS message for the named test case. */
#define TEST_PASS(name) \
    printf("PASS  %s\n", (name))

#endif /* INFINITY_TEST_HARNESS_H */
