-- Source of our final TWAP point (the TWAP at the window end). `twap_source` is the source of
-- the price-to-beat point. The M1 gate counts BTC windows whose final point came from PolyBolt.
-- Rows written before this column existed stay null: they never count toward the gate.
alter table windows add column final_twap_source text;

-- Heartbeat of a running session: the last time it was known alive. A session that ended
-- without a clean shutdown (crash, kill -9, power loss) is closed at the next start at this time.
alter table sessions add column last_seen_ms bigint;
-- Machine sleep inside the session; recorded time excludes it.
alter table sessions add column slept_ms bigint not null default 0;

-- How long a gap lasted, when known (a machine sleep).
alter table feed_gaps add column duration_ms bigint;

-- A window's status only moves forward: an upsert from a restarted or stale engine never
-- regresses it. `complete` outranks `timed_out`, so a late final price still completes a window.
create function window_status_rank(status text) returns int
language sql immutable as $$
  select case status
    when 'upcoming' then 0
    when 'live' then 1
    when 'ended' then 2
    when 'resolved' then 3
    when 'timed_out' then 4
    when 'complete' then 5
    else -1
  end
$$;
