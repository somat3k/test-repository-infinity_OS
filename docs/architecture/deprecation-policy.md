# Deprecation Policy and Migration Tooling

**Status:** `[x]` complete  
**Epic:** L — Layered Module Interfaces  
**Owner:** Architecture team

---

## 1. Purpose

This document defines the **deprecation process**, **semver rules**, and
**migration tooling** for all stable cross-layer APIs in infinityOS.

All stable APIs are published as Rust traits in the `ify-interfaces` crate
and versioned using `InterfaceVersion` constants in
`ify_interfaces::versioning`.

---

## 2. Semantic Versioning Rules for Public Contracts

infinityOS follows [Semantic Versioning 2.0.0](https://semver.org/) for all
stable interface versions.  The version triple is `MAJOR.MINOR.PATCH`.

| Change type | Version bump | Notes |
|-------------|-------------|-------|
| Add an optional method with a default implementation | `PATCH` | Callers unaffected |
| Add a non-breaking documentation improvement | `PATCH` | No API change |
| Add a new required method | `MAJOR` | All existing impls break |
| Remove or rename any public method | `MAJOR` | All callers break |
| Change any method signature | `MAJOR` | All callers break |
| Deprecate a method (method still present, `#[deprecated]` added) | `MINOR` | Callers warned but not broken |
| Add a new associated type with a default | `MINOR` | Existing impls unaffected |
| Add a new associated type without a default | `MAJOR` | All existing impls break |
| Change an error type | `MAJOR` | Callers that pattern-match break |
| Tighten a precondition (e.g. add a validation that previously succeeded) | `MAJOR` | Existing callers may panic |
| Relax a precondition | `MINOR` | Always backward compatible |

### 2.1 Compatibility check

Use `InterfaceVersion::is_compatible_with` to verify compatibility at runtime:

```rust
use ify_interfaces::versioning::{InterfaceVersion, EVENT_BUS_API_VERSION};

fn check_compat(consumer_expects: InterfaceVersion) {
    assert!(
        consumer_expects.is_compatible_with(&EVENT_BUS_API_VERSION),
        "EventBusApi version mismatch: consumer expects {consumer_expects}, \
         runtime provides {EVENT_BUS_API_VERSION}",
    );
}
```

---

## 3. Deprecation Process

### Step 1 — Mark as deprecated

Add `#[deprecated(since = "X.Y.Z", note = "...")]` to the method:

```rust
#[deprecated(
    since = "1.1.0",
    note = "Use `entries_after(cursor)` for efficient pagination instead."
)]
fn all_entries(&self) -> Vec<Self::Entry>;
```

Bump the interface version constant by `MINOR` (e.g. `1.0.0` → `1.1.0`).

### Step 2 — Publish migration guide

Create or update a section in this document (see §5) explaining:
- Why the method is deprecated.
- Which replacement to use.
- Code before and after migration.

### Step 3 — Update reference implementation

Add a `#[allow(deprecated)]` annotation to the concrete impl so it continues
to compile.  Add a log warning in the body:

```rust
#[allow(deprecated)]
fn all_entries(&self) -> Vec<Self::Entry> {
    tracing::warn!(
        "all_entries() is deprecated since 1.1.0; \
         use entries_after(cursor) instead"
    );
    // ... original implementation ...
}
```

### Step 4 — Remove after the grace period

The **grace period** is **two minor releases** after the deprecation release.
After the grace period:

1. Remove the method from the trait.
2. Bump the interface version constant by `MAJOR`.
3. Remove all `#[allow(deprecated)]` annotations from impls.
4. Update all callers in the workspace to use the replacement.

---

## 4. Migration Tooling

### 4.1 Compiler warnings

Rust's `#[deprecated]` attribute automatically emits `E0063`-style warnings at
every call site.  Enable `#[deny(deprecated)]` in CI to turn these into hard
errors after the grace period ends:

```toml
# In .cargo/config.toml or per-crate rustflags:
[target.'cfg(all())']
rustflags = ["-D", "deprecated"]
```

### 4.2 Semver audit script

Run the following to detect any interface constant that no longer matches the
declared version:

```sh
cd runtime-rust && cargo test -p ify-interfaces -- versioning
```

This executes the `api_version_constants_are_stable` test which asserts that
all constants remain at their expected `major` version.

### 4.3 Compatibility regression test

After bumping any interface version, add a test asserting the new version
satisfies backward-compatibility rules:

```rust
#[test]
fn event_bus_v1_1_compat_with_v1_0() {
    let v1_0 = InterfaceVersion::new(1, 0, 0);
    let v1_1 = InterfaceVersion::new(1, 1, 0);
    assert!(v1_0.is_compatible_with(&v1_1));   // consumer on 1.0 works with 1.1
    assert!(!v1_1.is_compatible_with(&v1_0));  // consumer on 1.1 does NOT work with 1.0
}
```

Place these in `ify-interfaces/src/versioning.rs`.

---

## 5. Migration Guides

_No deprecations are currently active.  This section will be populated as
methods are deprecated._

### Template

```markdown
### <Method name> (deprecated since X.Y.Z, removed in Z.0.0)

**Reason:** <why it was removed>

**Before:**
\```rust
bus.old_method(arg1, arg2);
\```

**After:**
\```rust
bus.new_method(arg1);  // arg2 is now inferred automatically
\```
```

---

## 6. Interface-Breaking Change Approval Process

Breaking changes (MAJOR version bumps) require:

1. **Contract diff** reviewed and approved by at least one cross-layer owner
   (see `docs/governance/interface-compatibility-policy.md`).
2. **Migration guide** in §5 of this document, covering all affected callers.
3. **Compatibility tests** in `ify-interfaces` verifying old-major fails and
   new-major passes.
4. **Grace period** announced in the PR description with a target removal milestone.

---

## 7. References

- [`layer-interfaces.md`](layer-interfaces.md) — canonical IDL for all cross-layer APIs
- [`docs/governance/interface-compatibility-policy.md`](../governance/interface-compatibility-policy.md) — review checklist
- `runtime-rust/crates/ify-interfaces/src/versioning.rs` — version constants
- [Semantic Versioning 2.0.0](https://semver.org/)
