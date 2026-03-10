/**
 * @file replication.c
 * @brief infinityOS Kernel — Replication Kernel Implementation
 *
 * Each replica is a task-scoped micro-kernel instance consisting of:
 *   - A private ify_arena_t for task-local allocations.
 *   - A private ify_scheduler_t for isolated task dispatch.
 *   - A lifecycle state machine (CREATED → RUNNING → DRAINING → DESTROYED).
 *
 * Replication policies control arena caps, concurrency limits, and
 * automatic destruction when all tasks complete.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#include <stdlib.h>
#include <string.h>

#include "internal.h"
#include "../include/infinity/replication.h"

/* --------------------------------------------------------------------------
 * Replica table
 * ------------------------------------------------------------------------ */

typedef struct {
    ify_replica_id_t     id;
    ify_replica_state_t  state;
    ify_replica_policy_t policy;
    ify_dimension_id_t   dimension_id;
    ify_task_id_t        owner_task_id;
    ify_arena_t         *arena;
    ify_scheduler_t     *scheduler;
} replica_slot_t;

static replica_slot_t   g_replicas[IFY_REPLICA_MAX];
static pthread_mutex_t  g_rep_lock   = PTHREAD_MUTEX_INITIALIZER;
static uint32_t         g_rep_next_id = 1;

/* --------------------------------------------------------------------------
 * Helpers
 * ------------------------------------------------------------------------ */

static replica_slot_t *rep_find(ify_replica_id_t id) {
    for (int i = 0; i < IFY_REPLICA_MAX; i++) {
        if (g_replicas[i].id == id &&
            g_replicas[i].state != IFY_REPLICA_FREE &&
            g_replicas[i].state != IFY_REPLICA_DESTROYED) {
            return &g_replicas[i];
        }
    }
    return NULL;
}

static replica_slot_t *rep_free_slot(void) {
    for (int i = 0; i < IFY_REPLICA_MAX; i++) {
        if (g_replicas[i].state == IFY_REPLICA_FREE ||
            g_replicas[i].state == IFY_REPLICA_DESTROYED) {
            return &g_replicas[i];
        }
    }
    return NULL;
}

/* --------------------------------------------------------------------------
 * ify_replica_create
 * ------------------------------------------------------------------------ */

ify_status_t ify_replica_create(const ify_replica_opts_t *opts,
                                 ify_replica_id_t *out_id) {
    if (opts == NULL || out_id == NULL) {
        return IFY_ERR_INVALID_ARG;
    }

    pthread_mutex_lock(&g_rep_lock);

    replica_slot_t *slot = rep_free_slot();
    if (slot == NULL) {
        pthread_mutex_unlock(&g_rep_lock);
        return IFY_ERR_OVERFLOW;
    }

    /* Determine arena capacity. */
    size_t arena_cap = (opts->policy.arena_cap_bytes != 0)
                       ? opts->policy.arena_cap_bytes
                       : (1u << 20); /* 1 MiB default */

    pthread_mutex_unlock(&g_rep_lock);

    /* Allocate arena and scheduler outside the lock to avoid nesting. */
    ify_arena_t *arena = ify_arena_create(arena_cap);
    if (arena == NULL) {
        return IFY_ERR_OUT_OF_MEMORY;
    }

    uint32_t max_tasks = (opts->policy.max_tasks != 0) ? opts->policy.max_tasks : 32u;
    ify_scheduler_opts_t sched_opts;
    memset(&sched_opts, 0, sizeof(sched_opts));
    sched_opts.dimension_id   = opts->dimension_id;
    sched_opts.max_concurrent = max_tasks;

    ify_scheduler_t *sched = ify_scheduler_create(&sched_opts);
    if (sched == NULL) {
        ify_arena_destroy(arena);
        return IFY_ERR_OUT_OF_MEMORY;
    }

    pthread_mutex_lock(&g_rep_lock);

    /* Re-check that the slot is still free (paranoia for multi-threaded). */
    if (slot->state != IFY_REPLICA_FREE &&
        slot->state != IFY_REPLICA_DESTROYED) {
        /* Slot was taken; find another. */
        slot = rep_free_slot();
        if (slot == NULL) {
            pthread_mutex_unlock(&g_rep_lock);
            ify_scheduler_destroy(sched);
            ify_arena_destroy(arena);
            return IFY_ERR_OVERFLOW;
        }
    }

    memset(slot, 0, sizeof(*slot));
    slot->id            = (ify_replica_id_t)g_rep_next_id++;
    slot->state         = IFY_REPLICA_CREATED;
    slot->policy        = opts->policy;
    slot->dimension_id  = opts->dimension_id;
    slot->owner_task_id = opts->owner_task_id;
    slot->arena         = arena;
    slot->scheduler     = sched;

    *out_id = slot->id;

    pthread_mutex_unlock(&g_rep_lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_replica_destroy
 * ------------------------------------------------------------------------ */

ify_status_t ify_replica_destroy(ify_replica_id_t id) {
    if (id == IFY_REPLICA_INVALID) {
        return IFY_OK;
    }

    pthread_mutex_lock(&g_rep_lock);
    replica_slot_t *slot = rep_find(id);
    if (slot == NULL) {
        pthread_mutex_unlock(&g_rep_lock);
        return IFY_ERR_NOT_FOUND;
    }
    slot->state = IFY_REPLICA_DRAINING;
    ify_scheduler_t *sched = slot->scheduler;
    ify_arena_t     *arena = slot->arena;
    slot->scheduler = NULL;
    slot->arena     = NULL;
    pthread_mutex_unlock(&g_rep_lock);

    /* Destroy scheduler (drains pending tasks). */
    ify_scheduler_destroy(sched);
    ify_arena_destroy(arena);

    pthread_mutex_lock(&g_rep_lock);
    slot = rep_find(id);
    if (slot != NULL) {
        slot->state = IFY_REPLICA_DESTROYED;
    }
    pthread_mutex_unlock(&g_rep_lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_replica_submit
 * ------------------------------------------------------------------------ */

ify_status_t ify_replica_submit(ify_replica_id_t  id,
                                 ify_task_fn_t     fn,
                                 void             *arg,
                                 ify_priority_t    priority,
                                 ify_task_id_t    *out_tid) {
    if (id == IFY_REPLICA_INVALID || fn == NULL) {
        return IFY_ERR_INVALID_ARG;
    }

    pthread_mutex_lock(&g_rep_lock);
    replica_slot_t *slot = rep_find(id);
    if (slot == NULL) {
        pthread_mutex_unlock(&g_rep_lock);
        return IFY_ERR_NOT_FOUND;
    }
    if (slot->state == IFY_REPLICA_DRAINING) {
        pthread_mutex_unlock(&g_rep_lock);
        return IFY_ERR_INVALID_ARG;
    }
    slot->state = IFY_REPLICA_RUNNING;
    ify_scheduler_t *sched = slot->scheduler;
    pthread_mutex_unlock(&g_rep_lock);

    return ify_scheduler_submit(sched, fn, arg, priority, out_tid);
}

/* --------------------------------------------------------------------------
 * ify_replica_state
 * ------------------------------------------------------------------------ */

ify_status_t ify_replica_state(ify_replica_id_t id, ify_replica_state_t *out) {
    if (out == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    pthread_mutex_lock(&g_rep_lock);
    replica_slot_t *slot = rep_find(id);
    ify_status_t rc;
    if (slot == NULL) {
        rc = IFY_ERR_NOT_FOUND;
    } else {
        *out = slot->state;
        rc   = IFY_OK;
    }
    pthread_mutex_unlock(&g_rep_lock);
    return rc;
}

/* --------------------------------------------------------------------------
 * ify_replica_arena / ify_replica_scheduler
 * ------------------------------------------------------------------------ */

ify_arena_t *ify_replica_arena(ify_replica_id_t id) {
    pthread_mutex_lock(&g_rep_lock);
    replica_slot_t *slot = rep_find(id);
    ify_arena_t *a = (slot != NULL) ? slot->arena : NULL;
    pthread_mutex_unlock(&g_rep_lock);
    return a;
}

ify_scheduler_t *ify_replica_scheduler(ify_replica_id_t id) {
    pthread_mutex_lock(&g_rep_lock);
    replica_slot_t *slot = rep_find(id);
    ify_scheduler_t *s = (slot != NULL) ? slot->scheduler : NULL;
    pthread_mutex_unlock(&g_rep_lock);
    return s;
}
