# Test fixtures (real captures, 2026-10-26, from Serbia)

Line-oriented fixtures are JSONL: `{"recv_ms": <local receive epoch ms>, "raw": "<exact frame text>"}`.

| File | Contents |
|---|---|
| `clob/live_window.jsonl` | First 1,500 frames of the CLOB market WebSocket for live window `btc-updown-5m-1793031000`. Starts with the snapshot array, then `price_change`, `book`, `last_trade_price` and `best_bid_ask` frames, `PONG`, and a dynamic subscribe snapshot for the next window. |
| `clob/after_window_end.jsonl` | Frames after the window ended: `tick_size_change` (duplicated), `new_market` (broadcast for unrelated markets) |
| `clob/market_resolved.jsonl` | One real `market_resolved` frame |
| `rtds/boundary.jsonl` | Legacy RTDS (`wss://ws-live-data.polymarket.com`) around window end 1793039400. Starts with an empty frame `""`, then snapshots and updates for `crypto_prices_twap_sixty`, `crypto_prices_chainlink` and `crypto_prices`. The BTC TWAP point at ts 1793039400000 has value 80121.11048035804, which equals Gamma `finalPrice`. |
| `gamma/btc_1793029800.json` | Resolved window: `eventMetadata` has `priceToBeat` 80584.595188212 and `finalPrice` 80399.2702888047; Down won |
| `gamma/btc_1793031000.json`, `gamma/btc_1793031300.json` | Live and next windows (no `eventMetadata`) |
| `gamma/eth_sample.json`, `gamma/sol_sample.json` | ETH and SOL events |
| `gamma/bad.json` | 404 body for an unknown slug |
| `data_api/resolution_resolved.json` | `/v2/resolutions` for a resolved window. `payouts` is `[Up, Down]` in micro-USDC. |
| `data_api/resolution_pending.json` | `{"data":[]}` for an unresolved window |
| `misc/geoblock.json` | `/api/geoblock` response (the IP is replaced with a documentation address) |
| `cex/binance.jsonl` | Binance combined stream: `bookTicker` and `trade` for btc/eth/sol |
| `cex/coinbase.jsonl` | Coinbase Exchange feed: the `subscriptions` ack, then `ticker` and `heartbeat` for BTC/ETH/SOL/USDT-USD |
| `polybolt/asyncapi_examples.jsonl` | `{"name","raw"}` lines taken from the official AsyncAPI examples. These are not live captures; credentials are needed for live ones. |
