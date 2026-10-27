# M1 follow-ups (deferred review findings)

Collected 2026-10-27 from the per-task reviews and the final whole-branch review of branch `m1-recorder`.
None blocks M1 recording. Most are hardening work; pick them up before M2 (replay) or M3 (paper trading) as noted.

## Final review — minor findings (not fixed)

- **Reader discards everything already decoded on a corruption error and loads whole hourly files into memory** (`backend/crates/recorder/src/reader.rs:38`)
- **The file-name session tag is the UUIDv7 timestamp prefix, so it is not unique per session** (`backend/crates/recorder/src/writer.rs:39`)
- **Records dropped on a full channel leave no marker in the raw stream** (`backend/crates/recorder/src/task.rs:43`)
- **Binary WebSocket frames are thrown away without being recorded** (`backend/crates/feeds/src/runners/mod.rs:333`)
- **WS market_resolved can latch the winner and stop polling the Data API, the stated authority** (`backend/crates/engine/src/engine.rs:129`)
- **A missing boundary TWAP point silently drops a window from verification** (`backend/crates/engine/src/oracle.rs:50`)
- **Book snapshots applied without ordering; resyncs are frequent and each is written as a feed gap** (`backend/crates/engine/src/engine.rs:80`)
- **No Host or Origin validation: DNS rebinding can drive the control endpoints and /api/ws is readable cross-origin** (`backend/crates/api/src/lib.rs:46`)
- **Shutdown flushes series rows but not the live windows' latest coverage_secs** (`backend/crates/traderd/src/shell.rs:198`)
- **`polybolt = true` without credentials only logs a warning and runs without PolyBolt** (`backend/crates/traderd/src/main.rs:107`)
- **Config is barely validated; bad values panic after the session is open or hammer remote endpoints** (`backend/crates/traderd/src/config.rs:105`)
- **Every BookResync becomes a feed_gaps row, which floods the gap log** (`backend/crates/traderd/src/shell.rs:339`)
- **Stale greying (filter + opacity on <main>) also hits the Kill/Resume dialog and moves its fixed backdrop** (`frontend/src/styles/app.css:165`)
- **Ack checkbox state survives a change of stop reason while the Resume dialog is open** (`frontend/src/components/ControlPanel.tsx:92`)
- **Chart lines interpolate across recording gaps and null samples, hiding gaps the M1 gate needs to explain** (`frontend/src/components/WindowChart.tsx:179`)
- **The series query cache is shared between the sparkline (staleTime Infinity) and the detail page, so the sparkline can show a gap** (`frontend/src/components/WindowCard.tsx:19`)
- **The e2e smoke test resumes backend safety stops against the real daemon** (`frontend/e2e/smoke.spec.ts:14`)
- **Control state isn't persisted: Kill or manual pause is silently undone by an app restart** (`backend/crates/engine/src/control.rs`)

## Backend per-task reviews — deferred minors

Some of these were fixed later by the final-review fix wave (commits aa3d3fe..e993ce6); check the code before acting on one.

- T1: WindowId::parse_slug accepts non-canonical starts ("+1793031000", negative multiples of 300) because i64::parse is lenient; slug() would not round-trip.
- T1: core tests only exercise Btc symbols and a few from_any forms; no all-assets/all-forms from_any test and no serde test pinning Outcome to "Up"/"Down".
- T1: docker-compose publishes 127.0.0.1:5433:5432 instead of the brief's 5433:5432 (benign loopback hardening, documented by implementer).
- T2: task.rs publish_file allocates a String and locks a mutex on every record (~400/s); only update when current_file changes.
- T2: writer rotation keys on rec.t, so records jittering across an hour boundary (several feeds, channel reordering) finish and reopen a new -n file each flip; consider only rotating forward.
- T2: file write/zstd/sync_all run synchronously inside the async recorder task; a slow fsync on rotation stalls a runtime worker (consider spawn_blocking or a dedicated thread).
- T2: reader.rs buffers the whole decompressed file in memory and counts blank lines as bad_lines; fine for M1, stream later for replay of large hours.
- T2: thiserror dep unused in raki-recorder; no test covers the writer I/O-error path in run() (stats/dropped after failure).
- T3: parse_clob array handling is all-or-nothing; one malformed object in a snapshot array discards the valid events of its siblings (raw is still recorded).
- T3: rtds/polybolt Frame deserializes `payload` eagerly as OraclePayload, so an ignorable frame (other topic/channel) with a differently shaped payload yields Err instead of Ok([])/Other; parse payload lazily (RawValue) after routing.
- T3: test nits in parsers.rs: clob_fixture_parses_without_errors reassigns `events` oddly and only checks after_window_end non-empty; polybolt_examples ends with a silent `_ => {}` arm.
- T4: RTDS "stale data" idle check counts WS control frames (runners/mod.rs:315), not text messages; data that stops while server pings continue faster than 15 s would never reconnect
- T4: CLOB sends no PING / no PONG deadline while unsubscribed (clob.rs:162,167), a live-justified deviation; an idle CLOB socket has no liveness check except the reconnect broadcast
- T4: shutdown path calls on_disconnected on a connected feed (mod.rs:225 -> status.rs:113), so a clean stop counts as a reconnect
- T4: FeedReset goes through try_send (mod.rs:420), so a full event channel can drop it silently except for the throttled warn
- T4: PolyBolt frames are JSON-parsed twice (polybolt_seq at polybolt.rs:212, then parse_polybolt at :225)
- T4: `serde_json::to_string(..).expect("frame serializes")` is repeated in clob/coinbase/rtds/polybolt; clob's to_json helper could move to runners/mod.rs
- T4: no tests for PolyBolt close 4001/4008 at max backoff, or for a 1000 close with zero texts taking the normal backoff; Link::send (mod.rs:284) does not observe shutdown or reconnect while a write is stalled
- T5: gamma_window interpolates slug into the URL path unescaped (gamma.rs:2056); a slug with '/', '?' or '#' would change the request target
- T5: resolution() ignores the response's status field and does not check condition_id matches the requested one (data_api.rs:1882)
- T5: get + record_http + status-check sequence repeated in gamma/data_api/geoblock; could be one helper in rest/mod.rs
- T5: geoblock records non-JSON bodies as null, losing the capture (geoblock.rs:2107); PolyRest::new uses expect on client build (mod.rs:2214)
- T5: no test exercises real TLS or asserts the absent User-Agent header; no test for a non-2xx data-api/geoblock status
- T6: WS market_resolved (≈end+26s) latches winner before end+60s, so Data API /v2/resolutions (the stated authority) is never polled and a disagreement would go unseen (engine.rs:130, windows.rs:86,167)
- T6: coverage_secs only rides along with non-coverage changes; a shutdown mid-Live loses coverage since the last upsert (windows.rs:129)
- T6: a Book for a no-longer-desired token re-creates a book entry kept until the next desired-set change (engine.rs:81)
- T6: pause/kill during pending ack keeps stop_kind LossCap/ConfigHalt and drops the new reason, deviating from brief "-> Paused, Manual" (control.rs:92)
- T6: GammaFetched applied by event slug without checking gamma.slug/start match the tracked window (engine.rs:123, windows.rs:74)
- T7: upsert_window overwrites every nullable column (winner, resolved_at, gamma fields, status) with the incoming record, so a stale/restarted engine record can regress a latched resolution; consider coalesce or a latch guard when the engine (Task 8+) is wired
- T7: end_session ignores rows_affected, so ending an unknown session id silently succeeds
- T7: unused deps in raki-store (tracing, tokio) and test helper runs migrations on the admin `postgres` database; enum encoding mixes as_str() (asset filter, gap source) with serde helpers
- T7: no test for insert_series chunking above 5000 rows or for stats with an open session's now-based recorded_secs beyond >= 60
- T8: run_sleep_monitor detection path (sleep_events++, Sleep record, reconnect.send) is untested; only the shutdown path is; extract a step fn taking injected clock readings.
- T8: safety_at reports "clock offset X ms > max" when clock.ok=false but offset is within max (e.g. state inconsistent); also `#![allow(dead_code)]` on health module must be removed in Task 10.
- T8: a single failed geoblock check or NTP round flips status to unknown immediately (fail-safe but may flap trading gates on transient network errors); consider a short grace/retry.
- T9: unmatched routes/methods fall through to axum's empty-body 404/405 instead of the JSON {"error"} shape (add a router fallback).
- T9: tests do not cover the limit upper clamp (limit=9999 -> 500) or the WS closing when the client closes / watch sender drops.
- T9: send_control timeout also covers the reply wait, so a slow shell can execute pause/kill/resume while the client sees 503; consider documenting or distinguishing "unconfirmed" from "unavailable".
- T9: /api/ws does not check the Origin header (CORS does not apply to WebSocket); fine locally, add an Origin allow-check before the dashboard is reachable off-machine.
- T9: no Origin-header check on control POSTs (defence in depth beyond the Content-Type requirement); pair it with the /api/ws Origin check before off-machine access.
- T10: shell.rs on_event labels every non-first FeedReset gap "reconnect", but PolyBolt also emits FeedReset on a seq gap, so those rows are mislabelled
- T10: shell.rs on_control publishes the live snapshot after the reply; the test's live_rx Killed assert relies on current_thread scheduling, so publish before replying
- T10: shell.rs engine-loop test covers no Window->upsert_window, Series flush or BookResync->gap paths
- T10: store calls run inline in the engine loop with no per-call timeout, so a hung Postgres stalls ticks and control replies (implementer concern 5)

## Dashboard per-task reviews — deferred minors

Some of these were fixed later by the final-review fix wave (commits aa3d3fe..e993ce6); check the code before acting on one.

- T1: LiveSnapshot not imported in client.ts as the brief step 4 listed (YAGNI, Task 2 will import it).
- T1: getJson/postControl call res.json() unguarded, so a malformed 2xx body throws a raw SyntaxError rather than ApiError.
- T1: tests/client.test.ts asserts API_URL default, which fails if VITE_API_URL is set in the shell or a .env during test runs.
- T2: useLive.ts:52 newer() also gates pushed frames, so a backend clock step backwards or restart with earlier generated_ms silently drops live frames under a "live" badge; pushed frames should always win, only the seed should be conditional
- T2: __root.tsx:251 stale also true while status is 'connecting' with a seeded snapshot, so page is greyed on first load before the socket opens
- T2: useLive.test.tsx has no assertion for the 'content stale' greyed class, which the brief's review focus 1 calls for
- T2: __root.tsx:266 'to as "/"' cast and NAV typed as string bypass router type safety; replace with typed literals when Task 3 adds the routes
- T3: WindowCard.tsx:497 Link `to` cast `as '/'` hides the missing /windows/$slug route type; drop the cast once Task 4 lands the route.
- T3: health.tsx:721 and WindowCard.tsx:392 ignore query errors; GateProgress shows "Stats unavailable." and the sparkline seed fails silently, neither shows the error text.
- T3: ControlPanel.tsx:161 dialog has no initial focus, focus trap or focus restore; Esc is the only keyboard affordance.
- T3: health.tsx:770-801 repeats the `.field` dt/dd markup that WindowCard defines as `Field`; extract a shared component.
- T3: no test covers LivePage btc/eth/sol ordering or its empty state, and the sparkline test does not check a second live point being appended.
- T3: health.tsx reads health/control from the WS snapshot only, so the page stays on "Waiting for data" if the socket is down while /api/health would answer (design choice, not mandated).
- T4: WINDOW_SECS duplicated in windows.index.tsx and WindowChart.tsx; windows.$slug.tsx has local dash() duplicating fmt DASH handling
- T4: StatusBadge/MatchBadge/resolutionLatency/coveragePct live in a route file and are imported by another route; move to components/
- T4: WindowChart render (reference line, table view, fmtLocal) has no component test; only pure helpers are tested
- T4: slug interpolated unencoded into /api/windows/${slug} paths; detail back-link drops the asset filter
- T5: smoke countdown check reads cards.first() and can flake if that card already shows the static "ended, awaiting resolution" text at a window boundary
- T5: playwright.config.ts reuseExistingServer:true silently tests whatever app holds localhost:3000; document or check a dashboard marker before running
- T5: the non-RUNNING branch never exercises "PAUSED within 3 s after Pause" live (only via Task 3 unit tests); the report says so but README/CLAUDE.md claim the test "pauses trading"
- T5: playwright.config.ts reuseExistingServer:true silently tests any other server on :3000
- T5: detail page empty-series text says "(app was not running)" even for upcoming windows
- T5: price chart y-axis labels repeat when range is ~3 USD (ticks rounded to whole dollars)
