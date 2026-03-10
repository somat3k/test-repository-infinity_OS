/**
 * @file service_registry.h
 * @brief infinityOS Kernel — Service Registry
 *
 * The service registry maintains a table of named kernel services.  Each
 * service has a well-defined lifecycle (register → start → running → stop →
 * unregister) and provides a health-check callback.
 *
 * Crash-only restart semantics: if a service's run callback returns a
 * non-IFY_OK status, the registry applies the service's restart policy
 * (maximum retries, exponential back-off) before declaring the service
 * permanently failed.
 *
 * ABI stability: all structs include a @c _reserved padding field.
 *
 * @copyright infinityOS contributors
 * @license   See LICENSE in the repository root
 */

#ifndef INFINITY_SERVICE_REGISTRY_H
#define INFINITY_SERVICE_REGISTRY_H

#include <stdint.h>
#include "kernel.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --------------------------------------------------------------------------
 * Service handle
 * ------------------------------------------------------------------------ */

/** Maximum length (including NUL) of a service name. */
#define IFY_SERVICE_NAME_MAX 64

/** Maximum number of services that can be registered simultaneously. */
#define IFY_SERVICE_MAX 64

/** Opaque service handle returned by ify_service_register(). */
typedef uint32_t ify_service_id_t;

/** Sentinel value indicating an invalid/unregistered service. */
#define IFY_SERVICE_INVALID ((ify_service_id_t)0)

/* --------------------------------------------------------------------------
 * Service lifecycle states
 * ------------------------------------------------------------------------ */

typedef enum {
    IFY_SVC_UNREGISTERED = 0, /**< Slot is free.                */
    IFY_SVC_REGISTERED   = 1, /**< Registered, not yet started. */
    IFY_SVC_STARTING     = 2, /**< Start in progress.           */
    IFY_SVC_RUNNING      = 3, /**< Fully operational.           */
    IFY_SVC_STOPPING     = 4, /**< Stop in progress.            */
    IFY_SVC_FAILED       = 5, /**< Crashed; restart policy may recover it. */
    IFY_SVC_STOPPED      = 6, /**< Cleanly stopped.             */
} ify_svc_state_t;

/* --------------------------------------------------------------------------
 * Restart policy
 * ------------------------------------------------------------------------ */

/**
 * @brief Restart policy for a kernel service.
 *
 * When a service's run callback exits with an error the registry will
 * attempt to restart it up to @c max_restarts times, waiting
 * @c backoff_base_ms * 2^attempt milliseconds between attempts.
 */
typedef struct {
    /** Maximum restart attempts before marking the service as permanently
     *  failed.  0 disables automatic restart (crash-only, no recovery). */
    uint32_t max_restarts;
    /** Base back-off delay in milliseconds (doubles each attempt). */
    uint32_t backoff_base_ms;
    /** Reserved; must be zero-initialized. */
    uint8_t  _reserved[8];
} ify_restart_policy_t;

/** Default policy: up to 3 restarts with a 100 ms base back-off. */
#define IFY_RESTART_POLICY_DEFAULT \
    { .max_restarts = 3, .backoff_base_ms = 100, ._reserved = {0} }

/** No restart (crash-only): service is marked failed on any error. */
#define IFY_RESTART_POLICY_NONE \
    { .max_restarts = 0, .backoff_base_ms = 0, ._reserved = {0} }

/* --------------------------------------------------------------------------
 * Service callbacks
 * ------------------------------------------------------------------------ */

/**
 * @brief Service start callback.
 *
 * Called once when the service is started.  May allocate resources.
 *
 * @param svc_id   ID of this service.
 * @param ctx      Caller-supplied context pointer.
 * @return         IFY_OK on success, or a negative error code.
 */
typedef ify_status_t (*ify_svc_start_fn_t)(ify_service_id_t svc_id, void *ctx);

/**
 * @brief Service stop callback.
 *
 * Called once when the service is stopped (normal or forced).  Must
 * release all resources acquired during start.
 *
 * @param svc_id   ID of this service.
 * @param ctx      Caller-supplied context pointer.
 */
typedef void (*ify_svc_stop_fn_t)(ify_service_id_t svc_id, void *ctx);

/**
 * @brief Service health-check callback.
 *
 * Called periodically by the registry health monitor.
 *
 * @param svc_id   ID of this service.
 * @param ctx      Caller-supplied context pointer.
 * @return         IFY_OK if healthy, or a negative error code.
 */
typedef ify_status_t (*ify_svc_health_fn_t)(ify_service_id_t svc_id, void *ctx);

/* --------------------------------------------------------------------------
 * Service descriptor
 * ------------------------------------------------------------------------ */

/**
 * @brief Descriptor passed to ify_service_register().
 */
typedef struct {
    /** Human-readable service name; must be unique. */
    char name[IFY_SERVICE_NAME_MAX];
    /** Start callback; must not be NULL. */
    ify_svc_start_fn_t start_fn;
    /** Stop callback; must not be NULL. */
    ify_svc_stop_fn_t  stop_fn;
    /** Health-check callback; may be NULL (health checks disabled). */
    ify_svc_health_fn_t health_fn;
    /** Context pointer passed to every callback. */
    void *ctx;
    /** Restart policy applied on abnormal exit. */
    ify_restart_policy_t restart_policy;
    /** Reserved; must be zero-initialized. */
    uint8_t _reserved[16];
} ify_svc_descriptor_t;

/* --------------------------------------------------------------------------
 * Registry API
 * ------------------------------------------------------------------------ */

/**
 * @brief Initialize the service registry.
 *
 * Called internally by ify_kernel_init().
 *
 * @return IFY_OK on success, or a negative error code.
 */
ify_status_t ify_service_registry_init(void);

/**
 * @brief Shut down the service registry, stopping all running services.
 *
 * Called internally by ify_kernel_shutdown().
 */
void ify_service_registry_shutdown(void);

/**
 * @brief Register a new service.
 *
 * The service is not started; call ify_service_start() to start it.
 *
 * @param desc    Service descriptor; must not be NULL.
 * @param out_id  Output parameter for the assigned service ID; must not be NULL.
 * @return        IFY_OK on success, IFY_ERR_ALREADY_EXISTS if a service with
 *                the same name is already registered, IFY_ERR_OVERFLOW if the
 *                registry is full.
 */
ify_status_t ify_service_register(const ify_svc_descriptor_t *desc,
                                  ify_service_id_t *out_id);

/**
 * @brief Unregister a service.
 *
 * The service must be in STOPPED or FAILED state.
 *
 * @param id  Service to unregister.
 * @return    IFY_OK on success, IFY_ERR_NOT_FOUND, or IFY_ERR_INVALID_ARG
 *            if the service is still running.
 */
ify_status_t ify_service_unregister(ify_service_id_t id);

/**
 * @brief Start a registered service.
 *
 * Transitions the service from REGISTERED to RUNNING by invoking its
 * start callback.
 *
 * @param id  Service to start.
 * @return    IFY_OK on success, or a negative error code.
 */
ify_status_t ify_service_start(ify_service_id_t id);

/**
 * @brief Stop a running service.
 *
 * Transitions the service to STOPPED by invoking its stop callback.
 *
 * @param id  Service to stop.
 * @return    IFY_OK on success, or a negative error code.
 */
ify_status_t ify_service_stop(ify_service_id_t id);

/**
 * @brief Query the current state of a service.
 *
 * @param id   Service to query.
 * @param out  Output parameter; must not be NULL.
 * @return     IFY_OK on success, IFY_ERR_NOT_FOUND if unknown.
 */
ify_status_t ify_service_state(ify_service_id_t id, ify_svc_state_t *out);

/**
 * @brief Run the health-check callback for a service.
 *
 * @param id  Service to check.
 * @return    IFY_OK if healthy, IFY_ERR_NOT_FOUND, or the error returned
 *            by the health callback.
 */
ify_status_t ify_service_health_check(ify_service_id_t id);

/**
 * @brief Look up a service ID by name.
 *
 * @param name    NUL-terminated service name; must not be NULL.
 * @param out_id  Output parameter; must not be NULL.
 * @return        IFY_OK on success, IFY_ERR_NOT_FOUND if no match.
 */
ify_status_t ify_service_find(const char *name, ify_service_id_t *out_id);

#ifdef __cplusplus
}
#endif

#endif /* INFINITY_SERVICE_REGISTRY_H */
