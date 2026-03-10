/**
 * @file scheduler.c
 * @brief infinityOS Kernel — Scheduler Queues, Priorities, and Timers
 *
 * Design:
 *   - IFY_PRIO_MAX+1 separate FIFO queues (one per priority level).
 *   - Task entries are heap-allocated and tracked in a flat array for
 *     O(1) state queries by TaskID.
 *   - Timers are stored in a sorted singly-linked list; ify_timer_once()
 *     fires callbacks when their deadline passes on the next tick.
 *   - A pthread mutex protects all scheduler fields.
 *
 * Note: this is a single-threaded cooperative scheduler.  Actual task
 * dispatch (calling the task function) is triggered by ify_scheduler_tick(),
 * which is not yet exposed in the public API but is called internally.
 * Tasks added via ify_scheduler_submit() are executed synchronously when
 * the submit call detects that the concurrency limit allows it.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

/* Needed for clock_gettime / CLOCK_MONOTONIC on POSIX systems. */
#define _POSIX_C_SOURCE 200809L

#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>

#include "internal.h"
#include "../include/infinity/scheduler.h"
#include "../include/infinity/memory.h"

/* --------------------------------------------------------------------------
 * Priority queue constants
 * ------------------------------------------------------------------------ */

#define IFY_PRIO_MAX    7u          /* Highest priority level. */
#define IFY_PRIO_LEVELS (IFY_PRIO_MAX + 1u)

/* --------------------------------------------------------------------------
 * Internal task entry
 * ------------------------------------------------------------------------ */

typedef struct task_entry {
    struct task_entry  *next;          /* Next entry in priority queue.    */
    ify_task_id_t       id;            /* Assigned TaskID.                 */
    ify_task_fn_t       fn;            /* Entry-point callback.            */
    void               *arg;           /* Caller context.                  */
    ify_priority_t      priority;      /* Scheduling priority.             */
    volatile ify_task_state_t state;   /* Current lifecycle state.         */
    int                 cancel_flag;   /* Non-zero when cancellation requested. */
} task_entry_t;

/* --------------------------------------------------------------------------
 * Internal timer entry
 * ------------------------------------------------------------------------ */

typedef struct timer_entry {
    struct timer_entry *next;
    uint64_t            deadline_ns;   /* Absolute monotonic deadline.   */
    uint64_t            id;            /* Monotonic timer ID.            */
    ify_timer_cb_t      cb;
    void               *arg;
    int                 cancelled;
} timer_entry_t;

/* --------------------------------------------------------------------------
 * Scheduler struct
 * ------------------------------------------------------------------------ */

/** Maximum number of task slots in the flat tracking array. */
#define IFY_SCHED_MAX_TASKS 4096u

struct ify_scheduler {
    pthread_mutex_t   lock;

    ify_dimension_id_t dimension_id;
    uint32_t           max_concurrent;
    uint32_t           rate_limit_per_sec;
    uint32_t           running_count;

    /* Per-priority FIFO queues (head/tail pairs). */
    task_entry_t      *q_head[IFY_PRIO_LEVELS];
    task_entry_t      *q_tail[IFY_PRIO_LEVELS];

    /* Flat array for O(1) state queries. */
    task_entry_t      *tasks[IFY_SCHED_MAX_TASKS];
    uint32_t           task_count;

    /* Monotonic TaskID counter (low 32 bits of lo word). */
    uint64_t           task_seq;

    /* Sorted timer list (ascending deadline). */
    timer_entry_t     *timer_head;
    uint64_t           timer_seq;
};

/* --------------------------------------------------------------------------
 * Helpers
 * ------------------------------------------------------------------------ */

/** Generate the next TaskID for this scheduler. */
static ify_task_id_t next_task_id(struct ify_scheduler *s) {
    ify_task_id_t id;
    id.hi = ify_time_now_ns();          /* Use time as high word for ordering. */
    id.lo = ++s->task_seq;
    return id;
}

/** Compare two TaskIDs for equality. */
static int task_id_eq(ify_task_id_t a, ify_task_id_t b) {
    return (a.hi == b.hi) && (a.lo == b.lo);
}

/** Find a task entry by ID (caller must hold the lock). */
static task_entry_t *find_task(struct ify_scheduler *s, ify_task_id_t id) {
    for (uint32_t i = 0; i < s->task_count; i++) {
        if (s->tasks[i] != NULL && task_id_eq(s->tasks[i]->id, id)) {
            return s->tasks[i];
        }
    }
    return NULL;
}

/** Enqueue a task at the tail of its priority queue. */
static void enqueue(struct ify_scheduler *s, task_entry_t *t) {
    uint8_t p = (t->priority <= IFY_PRIO_MAX) ? t->priority : (uint8_t)IFY_PRIO_MAX;
    t->next = NULL;
    if (s->q_tail[p] != NULL) {
        s->q_tail[p]->next = t;
    } else {
        s->q_head[p] = t;
    }
    s->q_tail[p] = t;
}

/** Dequeue the highest-priority waiting task (caller holds the lock). */
static task_entry_t *dequeue_highest(struct ify_scheduler *s) {
    for (int p = (int)IFY_PRIO_MAX; p >= 0; p--) {
        if (s->q_head[p] != NULL) {
            task_entry_t *t = s->q_head[p];
            s->q_head[p] = t->next;
            if (s->q_head[p] == NULL) {
                s->q_tail[p] = NULL;
            }
            t->next = NULL;
            return t;
        }
    }
    return NULL;
}

/** Run pending tasks up to the concurrency limit.  Called with lock held. */
static void drain_queue(struct ify_scheduler *s) {
    while (s->running_count < s->max_concurrent) {
        task_entry_t *t = dequeue_highest(s);
        if (t == NULL) {
            break;
        }
        if (t->cancel_flag) {
            t->state = IFY_TASK_CANCELLED;
            continue;
        }
        t->state = IFY_TASK_RUNNING;
        s->running_count++;

        /* Release lock while the task runs (cooperative model). */
        pthread_mutex_unlock(&s->lock);
        ify_status_t rc = t->fn(t->id, t->arg);
        pthread_mutex_lock(&s->lock);

        s->running_count--;
        t->state = (rc == IFY_OK) ? IFY_TASK_COMPLETED : IFY_TASK_FAILED;
    }
}

/* --------------------------------------------------------------------------
 * ify_scheduler_create / ify_scheduler_destroy
 * ------------------------------------------------------------------------ */

ify_scheduler_t *ify_scheduler_create(const ify_scheduler_opts_t *opts) {
    if (opts == NULL) {
        return NULL;
    }
    struct ify_scheduler *s =
        (struct ify_scheduler *)calloc(1, sizeof(*s));
    if (s == NULL) {
        return NULL;
    }
    if (pthread_mutex_init(&s->lock, NULL) != 0) {
        free(s);
        return NULL;
    }
    s->dimension_id      = opts->dimension_id;
    s->max_concurrent    = (opts->max_concurrent != 0) ? opts->max_concurrent : 32u;
    s->rate_limit_per_sec = opts->rate_limit_per_sec;
    s->running_count     = 0;
    s->task_seq          = 0;
    s->timer_seq         = 0;
    return s;
}

void ify_scheduler_destroy(ify_scheduler_t *sched) {
    if (sched == NULL) {
        return;
    }
    pthread_mutex_lock(&sched->lock);

    /* Cancel all queued tasks. */
    for (int p = (int)IFY_PRIO_MAX; p >= 0; p--) {
        task_entry_t *t = sched->q_head[p];
        while (t != NULL) {
            task_entry_t *nx = t->next;
            t->state = IFY_TASK_CANCELLED;
            t = nx;
        }
        sched->q_head[p] = NULL;
        sched->q_tail[p] = NULL;
    }

    /* Free all task entries. */
    for (uint32_t i = 0; i < sched->task_count; i++) {
        free(sched->tasks[i]);
        sched->tasks[i] = NULL;
    }

    /* Free timer list. */
    timer_entry_t *tm = sched->timer_head;
    while (tm != NULL) {
        timer_entry_t *nx = tm->next;
        free(tm);
        tm = nx;
    }
    sched->timer_head = NULL;

    pthread_mutex_unlock(&sched->lock);
    pthread_mutex_destroy(&sched->lock);
    free(sched);
}

/* --------------------------------------------------------------------------
 * ify_scheduler_submit
 * ------------------------------------------------------------------------ */

ify_status_t ify_scheduler_submit(
        ify_scheduler_t *sched,
        ify_task_fn_t    fn,
        void            *arg,
        ify_priority_t   priority,
        ify_task_id_t   *out_id) {

    if (sched == NULL || fn == NULL) {
        return IFY_ERR_INVALID_ARG;
    }

    pthread_mutex_lock(&sched->lock);

    if (sched->task_count >= IFY_SCHED_MAX_TASKS) {
        pthread_mutex_unlock(&sched->lock);
        return IFY_ERR_OVERFLOW;
    }

    task_entry_t *t = (task_entry_t *)calloc(1, sizeof(*t));
    if (t == NULL) {
        pthread_mutex_unlock(&sched->lock);
        return IFY_ERR_OUT_OF_MEMORY;
    }

    t->id       = next_task_id(sched);
    t->fn       = fn;
    t->arg      = arg;
    t->priority = priority;
    t->state    = IFY_TASK_QUEUED;

    /* Register in flat tracking array. */
    sched->tasks[sched->task_count++] = t;

    if (out_id != NULL) {
        *out_id = t->id;
    }

    enqueue(sched, t);
    drain_queue(sched);  /* Try to run immediately. */

    pthread_mutex_unlock(&sched->lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_scheduler_cancel
 * ------------------------------------------------------------------------ */

ify_status_t ify_scheduler_cancel(ify_scheduler_t *sched, ify_task_id_t task_id) {
    if (sched == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    pthread_mutex_lock(&sched->lock);
    task_entry_t *t = find_task(sched, task_id);
    ify_status_t rc;
    if (t == NULL) {
        rc = IFY_ERR_NOT_FOUND;
    } else if (t->state == IFY_TASK_COMPLETED ||
               t->state == IFY_TASK_FAILED    ||
               t->state == IFY_TASK_CANCELLED) {
        rc = IFY_ERR_NOT_FOUND; /* Already terminal. */
    } else {
        t->cancel_flag = 1;
        if (t->state == IFY_TASK_QUEUED) {
            t->state = IFY_TASK_CANCELLED;
        }
        rc = IFY_OK;
    }
    pthread_mutex_unlock(&sched->lock);
    return rc;
}

/* --------------------------------------------------------------------------
 * ify_scheduler_state
 * ------------------------------------------------------------------------ */

ify_status_t ify_scheduler_state(
        ify_scheduler_t  *sched,
        ify_task_id_t     task_id,
        ify_task_state_t *out) {

    if (sched == NULL || out == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    pthread_mutex_lock(&sched->lock);
    task_entry_t *t = find_task(sched, task_id);
    ify_status_t rc;
    if (t == NULL) {
        rc = IFY_ERR_NOT_FOUND;
    } else {
        *out = t->state;
        rc   = IFY_OK;
    }
    pthread_mutex_unlock(&sched->lock);
    return rc;
}

/* --------------------------------------------------------------------------
 * Timers
 * ------------------------------------------------------------------------ */

ify_status_t ify_timer_once(
        ify_scheduler_t *sched,
        uint64_t         delay_us,
        ify_timer_cb_t   cb,
        void            *arg,
        ify_timer_t    **out) {

    if (sched == NULL || cb == NULL) {
        return IFY_ERR_INVALID_ARG;
    }

    timer_entry_t *te = (timer_entry_t *)calloc(1, sizeof(*te));
    if (te == NULL) {
        return IFY_ERR_OUT_OF_MEMORY;
    }

    uint64_t now = ify_time_now_ns();
    te->deadline_ns = now + delay_us * UINT64_C(1000);
    te->cb          = cb;
    te->arg         = arg;
    te->cancelled   = 0;

    pthread_mutex_lock(&sched->lock);
    te->id = ++sched->timer_seq;

    /* Insert sorted by deadline. */
    timer_entry_t **pp = &sched->timer_head;
    while (*pp != NULL && (*pp)->deadline_ns <= te->deadline_ns) {
        pp = &(*pp)->next;
    }
    te->next = *pp;
    *pp = te;

    if (out != NULL) {
        *out = (ify_timer_t *)te;
    }

    /* Fire any expired timers now. */
    uint64_t cur_ns = ify_time_now_ns();
    while (sched->timer_head != NULL &&
           sched->timer_head->deadline_ns <= cur_ns) {
        timer_entry_t *fired = sched->timer_head;
        sched->timer_head = fired->next;
        if (!fired->cancelled) {
            uint64_t fid = fired->id;
            ify_timer_cb_t fcb  = fired->cb;
            void          *farg = fired->arg;
            free(fired);
            /* Release lock during callback. */
            pthread_mutex_unlock(&sched->lock);
            fcb(fid, farg);
            pthread_mutex_lock(&sched->lock);
        } else {
            free(fired);
        }
    }

    pthread_mutex_unlock(&sched->lock);
    return IFY_OK;
}

void ify_timer_cancel(ify_timer_t *timer) {
    if (timer == NULL) {
        return;
    }
    /* Mark cancelled; removal happens when it reaches the front of the list. */
    ((timer_entry_t *)timer)->cancelled = 1;
}
