/**
 * @file replication.h
 * @brief infinityOS Kernel — Replication Kernel
 *
 * The replication kernel allows the creation of task-scoped micro-kernel
 * instances (replicas) for specified workloads.  Each replica has its own
 * scheduler and memory arena, isolated from the global kernel state.
 *
 * Replication policies govern when to create a replica, resource caps,
 * teardown behaviour, and CPU/memory pinning.
 *
 * ABI stability: all structs include a @c _reserved padding field.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_REPLICATION_H
#define INFINITY_REPLICATION_H

#include <stdint.h>
#include "kernel.h"
#include "memory.h"
#include "scheduler.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * Replica handle
 * ------------------------------------------------------------------------ */

/** Maximum number of replicas that may coexist. */
#define IFY_REPLICA_MAX 32

/** Opaque replica handle. */
typedef uint32_t ify_replica_id_t;

/** Sentinel value indicating an invalid replica. */
#define IFY_REPLICA_INVALID ((ify_replica_id_t)0)

/* --------------------------------------------------------------------------
 * Replica lifecycle states
 * ------------------------------------------------------------------------ */

typedef enum {
    IFY_REPLICA_FREE      = 0, /**< Slot available.              */
    IFY_REPLICA_CREATED   = 1, /**< Allocated, not yet running.  */
    IFY_REPLICA_RUNNING   = 2, /**< Tasks are being dispatched.  */
    IFY_REPLICA_DRAINING  = 3, /**< Accepting no new tasks.      */
    IFY_REPLICA_DESTROYED = 4, /**< Resources released.          */
} ify_replica_state_t;

/* --------------------------------------------------------------------------
 * Replication policy
 * ------------------------------------------------------------------------ */

/**
 * @brief Policy that controls when and how a replica is created and torn down.
 */
typedef struct {
    /** Maximum number of concurrent tasks in this replica; 0 for default. */
    uint32_t max_tasks;
    /** Memory cap for the replica's arena in bytes; 0 for default (1 MiB). */
    size_t   arena_cap_bytes;
    /** Automatically destroy the replica when all its tasks complete. */
    uint8_t  auto_destroy;
    /** Pin replica scheduler to a specific CPU (0-based); UINT8_MAX = no pin. */
    uint8_t  cpu_pin;
    /** Reserved; must be zero-initialized. */
    uint8_t  _reserved[14];
} ify_replica_policy_t;

/** Default policy: 32 tasks, 1 MiB arena, auto-destroy when idle. */
#define IFY_REPLICA_POLICY_DEFAULT \
    { .max_tasks = 32, .arena_cap_bytes = (1u << 20), \
      .auto_destroy = 1, .cpu_pin = UINT8_MAX, ._reserved = {0} }

/* --------------------------------------------------------------------------
 * Replica creation options
 * ------------------------------------------------------------------------ */

/**
 * @brief Options passed to ify_replica_create().
 */
typedef struct {
    /** Owning dimension. */
    ify_dimension_id_t   dimension_id;
    /** Task that owns / requests this replica. */
    ify_task_id_t        owner_task_id;
    /** Resource and teardown policy. */
    ify_replica_policy_t policy;
    /** Reserved; must be zero-initialized. */
    uint8_t              _reserved[16];
} ify_replica_opts_t;

/* --------------------------------------------------------------------------
 * Replication API
 * ------------------------------------------------------------------------ */

/**
 * @brief Create a new task-scoped replica kernel.
 *
 * @param opts    Creation options; must not be NULL.
 * @param out_id  Output parameter for the assigned replica ID; must not be NULL.
 * @return        IFY_OK on success, IFY_ERR_OVERFLOW if IFY_REPLICA_MAX is
 *                reached, or IFY_ERR_OUT_OF_MEMORY.
 */
ify_status_t ify_replica_create(const ify_replica_opts_t *opts,
                                ify_replica_id_t *out_id);

/**
 * @brief Destroy a replica and release all its resources.
 *
 * Cancels any pending tasks.  Blocks until the replica's scheduler has
 * drained.  Passing IFY_REPLICA_INVALID is a no-op.
 *
 * @param id  Replica to destroy.
 * @return    IFY_OK on success, IFY_ERR_NOT_FOUND if unknown.
 */
ify_status_t ify_replica_destroy(ify_replica_id_t id);

/**
 * @brief Submit a task to a replica's scheduler.
 *
 * @param id        Replica to submit to.
 * @param fn        Task entry point; must not be NULL.
 * @param arg       Context pointer passed to @p fn (may be NULL).
 * @param priority  Task priority.
 * @param out_tid   Output TaskID; may be NULL.
 * @return          IFY_OK on success, or a negative error code.
 */
ify_status_t ify_replica_submit(ify_replica_id_t  id,
                                ify_task_fn_t     fn,
                                void             *arg,
                                ify_priority_t    priority,
                                ify_task_id_t    *out_tid);

/**
 * @brief Query the current state of a replica.
 *
 * @param id   Replica to query.
 * @param out  Output parameter; must not be NULL.
 * @return     IFY_OK on success, IFY_ERR_NOT_FOUND if unknown.
 */
ify_status_t ify_replica_state(ify_replica_id_t id, ify_replica_state_t *out);

/**
 * @brief Return a pointer to a replica's private memory arena.
 *
 * The caller may use this arena for task-local allocations.  The arena is
 * destroyed when the replica is destroyed.
 *
 * @param id  Replica handle.
 * @return    Arena pointer, or NULL if the replica is unknown.
 */
ify_arena_t *ify_replica_arena(ify_replica_id_t id);

/**
 * @brief Return a pointer to a replica's private scheduler.
 *
 * @param id  Replica handle.
 * @return    Scheduler pointer, or NULL if the replica is unknown.
 */
ify_scheduler_t *ify_replica_scheduler(ify_replica_id_t id);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_REPLICATION_H */
