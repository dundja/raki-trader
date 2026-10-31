-- Shadow predictions and strategy v0 shadow decisions (docs/specs/2026-10-31-shadow-predictions.md).
-- Shadow rows never count toward the M2 or M3 gates.

-- The model's forecast of a window at a checkpoint (seconds before the end).
create table predictions (
  slug text not null,
  asset text not null,
  checkpoint_s integer not null,
  ts_ms bigint not null,
  p_up double precision not null,
  mu double precision not null,
  sd double precision not null,
  sigma double precision not null,
  ptb double precision not null,
  spot double precision not null,
  twap double precision,
  binance_mid_usd double precision,
  coinbase_mid double precision,
  up_bid numeric,
  up_ask numeric,
  down_bid numeric,
  down_ask numeric,
  model_id text not null,
  session_id uuid not null,
  primary key (slug, checkpoint_s)
);

-- Each strategy parameter set, by hash.
create table strategy_params (
  params_hash text primary key,
  strategy_id text not null,
  params jsonb not null,
  first_seen_ms bigint not null
);

-- One strategy evaluation per tick in the entry window.
create table shadow_evals (
  session_id uuid not null,
  seq bigint not null,
  strategy_id text not null,
  params_hash text not null,
  slug text not null,
  ts_ms bigint not null,
  secs_left integer not null,
  side text,
  model_prob double precision,
  market_prob numeric,
  best_ask numeric,
  walk_shares numeric,
  walk_avg_price numeric,
  walk_fee numeric,
  edge numeric,
  action text not null,
  reason text,
  primary key (session_id, seq)
);
create index shadow_evals_slug on shadow_evals (slug);

-- The simulated fill of an `enter` evaluation.
create table shadow_fills (
  session_id uuid not null,
  seq bigint not null,
  slug text not null,
  side text not null,
  decided_ms bigint not null,
  due_ms bigint not null,
  filled_ms bigint not null,
  limit_price numeric not null,
  shares numeric not null,
  cost numeric not null,
  fee numeric not null,
  status text not null,
  primary key (session_id, seq)
);
create index shadow_fills_slug on shadow_fills (slug);
