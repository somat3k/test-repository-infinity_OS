/**
 * @file scheduler.h
 * @brief infinityOS Kernel — Scheduler Queues, Priorities, and Timer Interface
 *
 * Provides cooperative and preemptive scheduling primitives for the infinityOS
 * kernel.  The scheduler manages task queues, priorities, and timers within a
 * single dimension context and is the sole entry point for task dispatch.
 *
 * Design principles:
 * - Priority-aware FIFO queues (8 priority levels).
 * - Cooperative yield points at well-defined suspension sites.
 * - Per-dimension quota enforcement (max concurrent tasks, rate limits).
 * - Deterministic shutdown: all active tasks are cancelled in priority order.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_SCHEDULER_H
#define INFINITY_SCHEDULER_H

#include <stdint.h>
#include "kernel.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * Priority levels
 * ------------------------------------------------------------------------ */

/**
 * @brief Task priority — higher value means higher priority.
 *
 * Use IFY_PRIO_* constants for portability.
 */
typedef uint8_t ify_priority_t;

#define IFY_PRIO_IDLE       ((ify_priority_t)0)  /**< Background / idle work. */
#define IFY_PRIO_LOW        ((ify_priority_t)1)  /**< Non-critical workloads.  */
#define IFY_PRIO_NORMAL     ((ify_priority_t)4)  /**< Default priority.        */
#define IFY_PRIO_HIGH       ((ify_priority_t)6)  /**< User-interactive tasks.  */
#define IFY_PRIO_CRITICAL   ((ify_priority_t)7)  /**< Safety / system tasks.   */

/* --------------------------------------------------------------------------
 * Task states
 * ------------------------------------------------------------------------ */

/** Discrete lifecycle states for a scheduled task. */
typedef enum {
    IFY_TASK_QUEUED    = 0, /**< Submitted, waiting for a worker. */
    IFY_TASK_RUNNING   = 1, /**< Currently executing.             */
    IFY_TASK_PAUSED    = 2, /**< Suspended at a yield point.      */
    IFY_TASK_CANCELLED = 3, /**< Cancellation requested.          */
    IFY_TASK_FAILED    = 4, /**< Terminated with an error.        */
    IFY_TASK_COMPLETED = 5, /**< Finished successfully.           */
} ify_task_state_t;

/* --------------------------------------------------------------------------
 * Scheduler handle
 * ------------------------------------------------------------------------ */

/** Opaque scheduler instance; one per dimension. */
typedef struct ify_scheduler ify_scheduler_t;

/**
 * @brief Scheduler creation options.
 */
typedef struct {
    /** Dimension that owns this scheduler. */
    ify_dimension_id_t dimension_id;
    /** Maximum number of concurrently executing tasks; 0 for default (32). */
    uint32_t max_concurrent;
    /** Maximum tasks per second (rate limit); 0 for unlimited. */
    uint32_t rate_limit_per_sec;
    /** Reserved for future use — must be zero-initialized. */
    uint8_t _reserved[32];
} ify_scheduler_opts_t;

/**
 * @brief Create a scheduler for the given dimension.
 *
 * @param opts  Creation options; must not be NULL.
 * @return      Pointer to new scheduler, or NULL on allocation failure.
 */
ify_scheduler_t *ify_scheduler_create(const ify_scheduler_opts_t *opts);

/**
 * @brief Destroy a scheduler and cancel all pending tasks.
 *
 * Blocks until all running tasks have exited their cancellation handler.
 * Passing NULL is a no-op.
 *
 * @param sched  Scheduler handle.
 */
void ify_scheduler_destroy(ify_scheduler_t *sched);

/* --------------------------------------------------------------------------
 * Task submission and control
 * ------------------------------------------------------------------------ */

/**
 * @brief Function signature for a task entry point.
 *
 * @param task_id  Unique identifier for this task invocation.
 * @param arg      Caller-supplied context pointer.
 * @return         IFY_OK on success, or a negative error code.
 */
typedef ify_status_t (*ify_task_fn_t)(ify_task_id_t task_id, void *arg);

/**
 * @brief Submit a task to the scheduler queue.
 *
 * @param sched     Scheduler handle; must not be NULL.
 * @param fn        Task entry point; must not be NULL.
 * @param arg       Context pointer passed to @p fn (may be NULL).
 * @param priority  Scheduling priority (IFY_PRIO_*).
 * @param out_id    Output parameter for the assigned TaskID; may be NULL.
 * @return          IFY_OK on success, IFY_ERR_OVERFLOW if the queue is full.
 */
ify_status_t ify_scheduler_submit(
    ify_scheduler_t    *sched,
    ify_task_fn_t       fn,
    void               *arg,
    ify_priority_t      priority,
    ify_task_id_t      *out_id);

/**
 * @brief Request cancellation of an in-flight task.
 *
 * The task is responsible for checking ify_task_is_cancelled() at yield
 * points and returning IFY_ERR_* when cancellation is detected.
 *
 * @param sched    Scheduler handle; must not be NULL.
 * @param task_id  TaskID to cancel.
 * @return         IFY_OK if the request was recorded, IFY_ERR_NOT_FOUND if the
 *                 task does not exist or has already completed.
 */
ify_status_t ify_scheduler_cancel(ify_scheduler_t *sched, ify_task_id_t task_id);

/**
 * @brief Query the current state of a task.
 *
 * @param sched    Scheduler handle; must not be NULL.
 * @param task_id  TaskID to query.
 * @param out      Output parameter; must not be NULL.
 * @return         IFY_OK on success, IFY_ERR_NOT_FOUND if unknown.
 */
ify_status_t ify_scheduler_state(
    ify_scheduler_t *sched,
    ify_task_id_t    task_id,
    ify_task_state_t *out);

/* --------------------------------------------------------------------------
 * Timer interface
 * ------------------------------------------------------------------------ */

/** Opaque timer handle. */
typedef struct ify_timer ify_timer_t;

/**
 * @brief Callback invoked when a timer fires.
 *
 * @param timer_id  Opaque timer identifier (monotonic counter).
 * @param arg       Caller-supplied context pointer.
 */
typedef void (*ify_timer_cb_t)(uint64_t timer_id, void *arg);

/**
 * @brief Schedule a one-shot timer callback after @p delay_us microseconds.
 *
 * @param sched     Scheduler handle; must not be NULL.
 * @param delay_us  Delay in microseconds.
 * @param cb        Callback to invoke; must not be NULL.
 * @param arg       Context pointer passed to @p cb (may be NULL).
 * @param out       Output handle; may be NULL if tracking is not required.
 * @return          IFY_OK on success, or a negative error code.
 */
ify_status_t ify_timer_once(
    ify_scheduler_t *sched,
    uint64_t         delay_us,
    ify_timer_cb_t   cb,
    void            *arg,
    ify_timer_t    **out);

/**
 * @brief Cancel a pending timer.
 *
 * No-op if the timer has already fired.  Passing NULL is a no-op.
 *
 * @param timer  Timer handle to cancel.
 */
void ify_timer_cancel(ify_timer_t *timer);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_SCHEDULER_H */
