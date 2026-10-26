-- One row per app start.
create table sessions (
  id uuid primary key,
  started_ms bigint not null,
  ended_ms bigint,
  end_reason text,
  version text not null,
  host text not null
);

-- One row per tracked window. `*_ours` come from our recording, `*_gamma` from Polymarket Gamma.
create table windows (
  slug text primary key,
  asset text not null,
  start_ts bigint not null,
  end_ts bigint not null,
  event_id text,
  condition_id text,
  up_token text,
  down_token text,
  price_to_beat_ours double precision,
  final_twap_ours double precision,
  outcome_ours text,
  price_to_beat_gamma double precision,
  final_price_gamma double precision,
  twap_match boolean,
  winner text,
  resolved_at bigint,
  resolution_seen_ms bigint,
  twap_source text,
  coverage_secs integer not null default 0,
  status text not null,
  session_id uuid not null,
  updated_ms bigint not null
);
create index windows_asset_start on windows (asset, start_ts desc);

-- Per-second samples. Book prices are exact decimals, oracle values are f64.
create table window_series (
  slug text not null,
  ts bigint not null,
  twap double precision,
  chainlink double precision,
  binance_mid_usd double precision,
  coinbase_mid double precision,
  up_bid numeric,
  up_ask numeric,
  down_bid numeric,
  down_ask numeric,
  primary key (slug, ts)
);

create table feed_gaps (
  id bigserial primary key,
  session_id uuid not null,
  source text not null,
  at_ms bigint not null,
  reason text not null
);

create table control_audit (
  id bigserial primary key,
  at_ms bigint not null,
  action text not null,
  ok boolean not null,
  from_mode text not null,
  to_mode text not null,
  reason text,
  error text
);
