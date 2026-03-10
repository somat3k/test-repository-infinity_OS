/**
 * @file service_registry.c
 * @brief infinityOS Kernel — Service Registry Implementation
 *
 * Implements a fixed-size table of named kernel services.  Each service
 * slot tracks lifecycle state and restart policy.
 *
 * Crash-only restart: when a service's start callback fails the registry
 * records the failure and, if max_restarts > 0, schedules retry attempts
 * with exponential back-off using nanosleep.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

/* Needed for nanosleep on POSIX systems. */
#define _POSIX_C_SOURCE 200809L

#include <string.h>
#include <time.h>
#include <stdlib.h>

#include "internal.h"
#include "../include/infinity/service_registry.h"

/* --------------------------------------------------------------------------
 * Registry table
 * ------------------------------------------------------------------------ */

typedef struct {
    ify_svc_descriptor_t desc;
    ify_service_id_t     id;
    ify_svc_state_t      state;
    uint32_t             restart_count;
} svc_slot_t;

static svc_slot_t        g_svc[IFY_SERVICE_MAX];
static pthread_mutex_t   g_svc_lock = PTHREAD_MUTEX_INITIALIZER;
static uint32_t          g_svc_next_id = 1;  /* IDs start at 1 (0 = invalid) */
static int               g_svc_initialized = 0;

/* --------------------------------------------------------------------------
 * ify_service_registry_init / ify_service_registry_shutdown
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_registry_init(void) {
    pthread_mutex_lock(&g_svc_lock);
    memset(g_svc, 0, sizeof(g_svc));
    g_svc_next_id    = 1;
    g_svc_initialized = 1;
    pthread_mutex_unlock(&g_svc_lock);
    return IFY_OK;
}

void ify_service_registry_shutdown(void) {
    pthread_mutex_lock(&g_svc_lock);
    if (!g_svc_initialized) {
        pthread_mutex_unlock(&g_svc_lock);
        return;
    }
    /* Stop all running services in reverse registration order. */
    for (int i = IFY_SERVICE_MAX - 1; i >= 0; i--) {
        svc_slot_t *s = &g_svc[i];
        if (s->state == IFY_SVC_RUNNING || s->state == IFY_SVC_STARTING) {
            s->state = IFY_SVC_STOPPING;
            if (s->desc.stop_fn != NULL) {
                pthread_mutex_unlock(&g_svc_lock);
                s->desc.stop_fn(s->id, s->desc.ctx);
                pthread_mutex_lock(&g_svc_lock);
            }
            s->state = IFY_SVC_STOPPED;
        }
    }
    memset(g_svc, 0, sizeof(g_svc));
    g_svc_initialized = 0;
    pthread_mutex_unlock(&g_svc_lock);
}

/* --------------------------------------------------------------------------
 * Internal helpers
 * ------------------------------------------------------------------------ */

static svc_slot_t *slot_by_id(ify_service_id_t id) {
    for (int i = 0; i < IFY_SERVICE_MAX; i++) {
        if (g_svc[i].id == id && g_svc[i].state != IFY_SVC_UNREGISTERED) {
            return &g_svc[i];
        }
    }
    return NULL;
}

static svc_slot_t *slot_free(void) {
    for (int i = 0; i < IFY_SERVICE_MAX; i++) {
        if (g_svc[i].state == IFY_SVC_UNREGISTERED) {
            return &g_svc[i];
        }
    }
    return NULL;
}

/* --------------------------------------------------------------------------
 * ify_service_register
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_register(const ify_svc_descriptor_t *desc,
                                   ify_service_id_t *out_id) {
    if (desc == NULL || out_id == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    if (desc->start_fn == NULL || desc->stop_fn == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    if (desc->name[0] == '\0') {
        return IFY_ERR_INVALID_ARG;
    }

    pthread_mutex_lock(&g_svc_lock);

    /* Check for duplicate name. */
    for (int i = 0; i < IFY_SERVICE_MAX; i++) {
        if (g_svc[i].state != IFY_SVC_UNREGISTERED &&
            strncmp(g_svc[i].desc.name, desc->name, IFY_SERVICE_NAME_MAX) == 0) {
            pthread_mutex_unlock(&g_svc_lock);
            return IFY_ERR_ALREADY_EXISTS;
        }
    }

    svc_slot_t *slot = slot_free();
    if (slot == NULL) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_OVERFLOW;
    }

    memcpy(&slot->desc, desc, sizeof(*desc));
    slot->id            = (ify_service_id_t)g_svc_next_id++;
    slot->state         = IFY_SVC_REGISTERED;
    slot->restart_count = 0;
    *out_id = slot->id;

    pthread_mutex_unlock(&g_svc_lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_service_unregister
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_unregister(ify_service_id_t id) {
    pthread_mutex_lock(&g_svc_lock);
    svc_slot_t *s = slot_by_id(id);
    if (s == NULL) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_NOT_FOUND;
    }
    if (s->state == IFY_SVC_RUNNING || s->state == IFY_SVC_STARTING) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_INVALID_ARG;
    }
    memset(s, 0, sizeof(*s));
    pthread_mutex_unlock(&g_svc_lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_service_start  (with crash-only restart)
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_start(ify_service_id_t id) {
    pthread_mutex_lock(&g_svc_lock);
    svc_slot_t *s = slot_by_id(id);
    if (s == NULL) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_NOT_FOUND;
    }
    if (s->state != IFY_SVC_REGISTERED && s->state != IFY_SVC_STOPPED &&
        s->state != IFY_SVC_FAILED) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_INVALID_ARG;
    }
    s->state         = IFY_SVC_STARTING;
    s->restart_count = 0;

    /* Attempt to start with retry loop. */
    uint32_t max_restarts   = s->desc.restart_policy.max_restarts;
    uint32_t backoff_base   = s->desc.restart_policy.backoff_base_ms;
    ify_svc_start_fn_t start_fn = s->desc.start_fn;
    ify_service_id_t   svc_id   = s->id;
    void              *ctx      = s->desc.ctx;

    for (uint32_t attempt = 0; ; attempt++) {
        pthread_mutex_unlock(&g_svc_lock);
        ify_status_t rc = start_fn(svc_id, ctx);
        pthread_mutex_lock(&g_svc_lock);

        /* Re-fetch slot in case the table changed during the unlock. */
        s = slot_by_id(svc_id);
        if (s == NULL) {
            return IFY_ERR_NOT_FOUND;
        }

        if (rc == IFY_OK) {
            s->state = IFY_SVC_RUNNING;
            pthread_mutex_unlock(&g_svc_lock);
            return IFY_OK;
        }

        s->restart_count++;
        if (attempt >= max_restarts) {
            s->state = IFY_SVC_FAILED;
            pthread_mutex_unlock(&g_svc_lock);
            return rc;
        }

        /* Exponential back-off: base * 2^attempt ms, capped at 30 s. */
        uint32_t backoff_ms = backoff_base;
        for (uint32_t j = 0; j < attempt && backoff_ms < 30000u; j++) {
            backoff_ms *= 2;
        }
        if (backoff_ms > 30000u) {
            backoff_ms = 30000u;
        }

        pthread_mutex_unlock(&g_svc_lock);
        if (backoff_ms > 0) {
            struct timespec ts = {
                (time_t)(backoff_ms / 1000u),
                (long)((backoff_ms % 1000u) * 1000000L)
            };
            nanosleep(&ts, NULL);
        }
        pthread_mutex_lock(&g_svc_lock);

        /* Re-fetch again after sleep. */
        s = slot_by_id(svc_id);
        if (s == NULL) {
            return IFY_ERR_NOT_FOUND;
        }
    }
}

/* --------------------------------------------------------------------------
 * ify_service_stop
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_stop(ify_service_id_t id) {
    pthread_mutex_lock(&g_svc_lock);
    svc_slot_t *s = slot_by_id(id);
    if (s == NULL) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_NOT_FOUND;
    }
    if (s->state != IFY_SVC_RUNNING) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_INVALID_ARG;
    }
    s->state = IFY_SVC_STOPPING;
    ify_svc_stop_fn_t stop_fn = s->desc.stop_fn;
    ify_service_id_t  svc_id  = s->id;
    void             *ctx     = s->desc.ctx;
    pthread_mutex_unlock(&g_svc_lock);

    stop_fn(svc_id, ctx);

    pthread_mutex_lock(&g_svc_lock);
    s = slot_by_id(svc_id);
    if (s != NULL) {
        s->state = IFY_SVC_STOPPED;
    }
    pthread_mutex_unlock(&g_svc_lock);
    return IFY_OK;
}

/* --------------------------------------------------------------------------
 * ify_service_state
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_state(ify_service_id_t id, ify_svc_state_t *out) {
    if (out == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    pthread_mutex_lock(&g_svc_lock);
    svc_slot_t *s = slot_by_id(id);
    ify_status_t rc;
    if (s == NULL) {
        rc = IFY_ERR_NOT_FOUND;
    } else {
        *out = s->state;
        rc   = IFY_OK;
    }
    pthread_mutex_unlock(&g_svc_lock);
    return rc;
}

/* --------------------------------------------------------------------------
 * ify_service_health_check
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_health_check(ify_service_id_t id) {
    pthread_mutex_lock(&g_svc_lock);
    svc_slot_t *s = slot_by_id(id);
    if (s == NULL) {
        pthread_mutex_unlock(&g_svc_lock);
        return IFY_ERR_NOT_FOUND;
    }
    ify_svc_health_fn_t hfn   = s->desc.health_fn;
    ify_service_id_t    svc_id = s->id;
    void               *ctx    = s->desc.ctx;
    pthread_mutex_unlock(&g_svc_lock);

    if (hfn == NULL) {
        return IFY_OK;  /* No health check means "assumed healthy". */
    }
    return hfn(svc_id, ctx);
}

/* --------------------------------------------------------------------------
 * ify_service_find
 * ------------------------------------------------------------------------ */

ify_status_t ify_service_find(const char *name, ify_service_id_t *out_id) {
    if (name == NULL || out_id == NULL) {
        return IFY_ERR_INVALID_ARG;
    }
    pthread_mutex_lock(&g_svc_lock);
    for (int i = 0; i < IFY_SERVICE_MAX; i++) {
        if (g_svc[i].state != IFY_SVC_UNREGISTERED &&
            strncmp(g_svc[i].desc.name, name, IFY_SERVICE_NAME_MAX) == 0) {
            *out_id = g_svc[i].id;
            pthread_mutex_unlock(&g_svc_lock);
            return IFY_OK;
        }
    }
    pthread_mutex_unlock(&g_svc_lock);
    return IFY_ERR_NOT_FOUND;
}
