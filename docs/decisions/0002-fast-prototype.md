# ADR-0002 — Reuse full pinned Compás for the first prototype

Status: accepted for prototype scope. Date: 2026-10-25. Authority: user asked to start working on a prototype ASAP; orchestrator selects reversible reuse strategy within that scope.

Reuse Compás commit 102d2d3b6fd049efcd670c5a11bcc304ee726612 (MIT), including existing Tauri UI and controller profiles. Preserve upstream copyright. Separate app identifier and disable upstream updater to prevent prototype replacement.

Alternative: extract engine first and build a new shell; this delays physical testing. Defer extraction until actual engine/UX constraints are measured. Inherited controller dispatch through frontend does not satisfy final Rust routing architecture; headphone channel routing and jog mappings remain open. No AI product functionality is added.

Spotify mixing cannot be delivered through the public developer API; partner access remains an external blocker. Continue local audio development and be explicit about this limitation.

Validation: engine tests and frontend build passed. Independent controller diff review passed. Native launch and physical tests pending at decision time. Revisit after hardware spike; never use this ADR to waive release/audio safety gates.
