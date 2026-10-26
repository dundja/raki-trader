# Polymarket 5-minute crypto markets: research report

_Researched 2026-10-26 by a multi-agent workflow (5 topic researchers, 2 adversarial fact-checkers, 1 completeness critic). Polymarket changes fast: re-verify anything marked here before relying on it. Fact-check corrections are listed first; where they conflict with a topic section, **the correction wins**._

## Corrections from fact-checking (read first)

These claims were refuted, outdated, or unverifiable when checked independently. Where they conflict with the topic sections below, these win.

- **UNVERIFIABLE**: Order book snapshot at T-56s (single sample): BTC favorite top ~1,239 shares at 0.67, 1c spread, $150-$700 within 3c. ETH thinner with a 4c spread. SOL almost empty. Expect only hundreds to a few thousand dollars fillable per BTC window.  
  Correction: I could not reproduce this. My /book requests failed: one returned HTTP 403, and a later try returned a non-JSON body. One useful related finding: CLOB reports minimum tick size 0.001 near extreme prices, and prices of 0.9995 were seen after the window ended, while Gamma still listed orderPriceMinTickSize 0.01. Read the tick size from CLOB, not Gamma. Treat depth as unknown until the bot logs books itself during paper trading.  
  Sources: <https://clob.polymarket.com/book?token_id=<token> (request failed during check)>, <https://clob.polymarket.com/clob-markets/0x6afe619355abc574ab4678da131b78ec3f1aec3058e042062611d080dbc5621f (mts 0.001)>

- **REFUTED**: Resolution latency: 15 sampled BTC windows had closedTime 78-80s after endDate. One was unresolved >133s after end. Bitquery reported median 26s and p90 85s (data to Aug 2026). Plan for 1.5-3+ minutes before capital is redeemable.  
  Correction: The Bitquery figures are correct: median 26s, p90 about 85s, data 1 Sep 2025 to 22 Aug 2026, verified 24 Aug 2026. Gamma closedTime is NOT a reliable measure of when resolution actually happened. It reads exactly end+80s for every window, including two that I watched stay unresolved for longer. Window 1793027400 ended 15:15:00, but at 15:19:13 Gamma still showed closedTime null and no finalPrice; later it was backfilled as 15:16:20. Window 1793027700 ended 15:20:00, and CLOB still showed closed=false and accepting_orders=true at 15:22:02; Gamma later recorded closedTime 15:21:20. Real resolution latency can exceed 2-4 minutes. Do not use closedTime for latency stats. Measure latency yourself from the CLOB closed/winner flags or on-chain resolution events. The '1.5-3+ min' budget is reasonable, with a longer tail.  
  Sources: Own live polling of CLOB /markets and Gamma events, 2026-10-26 15:19-15:36 UTC, <https://bitquery.io/investigations/polymarket-oracle-race>

- **REFUTED**: Taker delay is active (itode:true). Official docs say 250 ms; a third-party post claims it was raised from 50 to 150 ms on 2026-09-04, so the current value is uncertain. The original 500 ms delay was removed ~Feb 18 2026.  
  Correction: The value is not uncertain. Polymarket's official changelog says the crypto taker delay went from 250ms to 50ms on Aug 17 2026, then to 150ms on Sep 4 2026 at 14:00 UTC. The order-lifecycle page that says 250 ms is stale. Current value: 150 ms. The Feb 18 2026 removal of the 500 ms delay is backed only by secondary sources (Protos/cryptonews, 2026-02-20) and has no official changelog entry.  
  Sources: <https://docs.polymarket.com/changelog/predictions>, <https://docs.polymarket.com/concepts/order-lifecycle.md>, <https://cryptonews.net/news/market/32461495/>

- **REFUTED**: CLOB V2 went live 2026-04-28; V1 no longer works. The order struct changed (no nonce/feeRateBps; adds ms timestamp, metadata, builder) and the EIP-712 exchange domain version is '2'. Collateral moved to pUSD; API-only traders wrap via the Collateral Onramp. Official SDKs are TS and Python only, with no official Rust SDK, so Rust must implement signing itself or use a community crate.  
  Correction: The V2 facts are correct. The Rust part is wrong. Polymarket publishes an official Rust client, github.com/Polymarket/rs-clob-client-v2 (crate polymarket_client_sdk_v2, v0.8.0, authored by Polymarket Engineering, last pushed 2026-10-19). The Jul 17 2026 changelog also tells users to update their TypeScript, Python and Rust SDKs. The v2-migration page just doesn't mention it. Use the official crate; the V1 rs-clob-client is archived.  
  Sources: <https://github.com/Polymarket/rs-clob-client-v2>, <https://docs.polymarket.com/v2-migration>, <https://docs.polymarket.com/changelog/predictions (Jul 17)>

- **OUTDATED**: These markets apply a 250 ms taker delay (itode:true). Marketable orders are held 250 ms, re-validated, then matched or booked. They can't be cancelled while held. A submission can return status 'delayed'.  
  Correction: The delay is 150 ms since 2026-09-04 14:00 UTC (it was 250 ms until Aug 17, then 50 ms). The order-lifecycle page's 250 ms is stale. itode:true was confirmed live, and the hold, re-validation and no-cancel behaviour are correct. On 'delayed': the docs describe that status for seconds-delay markets such as sports, so it is unconfirmed whether crypto taker-delay orders return it. Handle it defensively.  
  Sources: <https://docs.polymarket.com/changelog/predictions>, <https://docs.polymarket.com/concepts/order-lifecycle.md>, Live clob-markets 2026-10-26

- **UNVERIFIABLE**: A new card-funded email account created after May 2026 should trade with signature type 3 and funder = deposit wallet; owner key presumably the Magic-held key (inferred).  
  Correction: No primary source says who the owner/signer of an email-account Deposit Wallet is. Supporting evidence: py-clob-client-v2 issue #52 (2026-05) shows fresh EOAs rejected with 'maker address not allowed, please use the deposit wallet flow', so sig type 3 with funder = deposit wallet is very likely right. Read the signer address in the Relayer API key UI or client.account before trusting the Magic assumption.  
  Sources: <https://docs.polymarket.com/trading/wallets-auth.md>, <https://github.com/Polymarket/py-clob-client-v2/issues/52>, <https://help.polymarket.com/en/articles/13364258-how-do-i-export-my-key>

- **REFUTED**: Safe cloud regions: non-listed countries (Serbia, Finland, Sweden, Austria, Spain...) plus IE and NL (API not restricted); AWS eu-west-1 Dublin documented closest; UK/DE/FR/IT/PL/US/SG/AU VPS will be close-only.  
  Correction: The non-listed countries and the close-only list are right. Calling IE and NL 'safe' is not supported. The official help center (2026-08-14) lists IE and NL as completely restricted and says VPN-style bypassing breaks ToS 2.1.4, while the developer docs say their API is unrestricted. A Railway community post (2026-03-30) also reports Polymarket blocked all Railway regions, including EU West Amsterdam. Lower-risk choices are VPS regions in countries absent from both lists, such as Finland, Sweden or Austria, or staying in Serbia. Treat Dublin as a latency option whose eligibility should be checked with Polymarket support and re-checked live via /api/geoblock.  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>, <https://help.polymarket.com/en/articles/13364163-geographic-restrictions>, <https://station.railway.com/feedback/additional-region-for-polymarket-trading-aef428dc>

- **UNVERIFIABLE**: Own on-chain analysis of 576 windows: BTC last-45s 0.92-0.95 won 95.2% of shares (~+1.4%/$ after fee), other bands mixed, ~$100k/day capacity, roughly calibrated.  
  Correction: This is the requester's own analysis, which I did not replicate. A 2-day, weekend-heavy sample of correlated windows cannot separate +1.4% from zero. Treat it as a hypothesis for the paper-trading phase.  
  Sources: <https://data-api.polymarket.com/trades>

## Contents

1. Mechanics and settlement of Polymarket 5-minute crypto Up/Down markets, with 15-minute, 4-hour and hourly variants for context (research date 2026-10-26)
2. Fees and economics of Polymarket 5-minute and 15-minute crypto Up/Down markets (as of 2026-10-26)
3. Technical API stack for a Rust Polymarket 5-minute crypto Up/Down bot (CLOB REST/WS, Gamma, RTDS/PolyBolt, auth, Rust SDK, rate limits, latency), verified 2026-10-26
4. Legal, access and account risk for a Serbia-based user running an automated Polymarket (international) trading bot, as of Oct 2026
5. Competition and strategy evidence for late-window high-probability ("endgame", sniping, latency arbitrage) strategies on Polymarket 5m/15m crypto Up/Down markets, as of 2026-10-26
6. Completeness critique (what we might have missed)


---

## 1. Mechanics and settlement of Polymarket 5-minute crypto Up/Down markets, with 15-minute, 4-hour and hourly variants for context (research date 2026-10-26)

### Summary

The biggest finding: since 7 Aug 2026, 5m/15m/4h Up/Down markets resolve on a Chainlink 60-second TWAP, not a spot snapshot. The 5m lookback was 30s at first and moved to 60s on 14 Aug 2026. Gamma lists the source as data.chain.link/streams/<asset>-usd-twap-60s-streams. Rule: Up if final TWAP >= price to beat, otherwise Down, so a tie resolves Up. In live data the price to beat of window N exactly equals the final TWAP of window N-1. The final value is the average over the last 60 seconds of the window, so at T-30s about half of it is already fixed. The strategy must therefore model the running 60s Chainlink TWAP, not Binance spot against a start print. Hourly (and reportedly daily) markets are different: they settle on Binance BTC/USDT candles.

Assets with 5m markets today: BTC, ETH, SOL, XRP, DOGE, HYPE, BNB, ZEC. All 8 also have 15m and 4h markets. Liquidity is very uneven. 24h series volume was about $12.3M for BTC, $314k for ETH and $221k for SOL, and $30k or less for the rest. Only BTC (maybe ETH/SOL) suits a high-volume bot.

Discovery: slugs are deterministic, `{asset}-updown-5m-{unix_start}` with start a multiple of 300 (15m: `-15m-` and multiples of 900; 4h: `-4h-` and multiples of 14400). Each window is created about 24h ahead and accepts orders from creation. Fetch with GET gamma-api.polymarket.com/events/slug/<slug>. The BTC 5m series is id 10684 (slug btc-up-or-down-5m). Listing by series_id returns stale old events, so compute slugs from the clock instead.

Settlement is automatic, with no UMA. Gamma shows automaticallyResolved=true, and /v2/resolutions shows status resolved with payouts recorded on-chain. In my sample, resolution came about 78-80s after window end, but one window was still unresolved after more than 130s. There is no cutoff before the window ends. The market keeps accepting orders after endDate until it resolves; at end+51s it was still trading at 0.975/0.025. Winnings are not auto-credited through the API: tokens must be redeemed (redeemPositions via the gasless Relayer), with no deadline.

Fees: crypto taker fee = shares × 0.07 × p × (1-p). At p=0.92 that is about 0.56% of cost, so breakeven win rate is about 92.5%. Makers pay nothing and get a 20% rebate. Crypto taker orders are held 150ms (since 4 Sep 2026) and cannot be cancelled during the hold. Minimum order is 5 shares; tick is 0.01, and resolved markets showed 0.001.

Data: RTDS price topics are legacy since 15 Sep 2026. PolyBolt (wss://ws-live-v2.polymarket.com/ws) streams price.crypto (Chainlink by default) and price.crypto.twap (60s), and needs CLOB API credentials. Direct Chainlink Data Streams access is paid. Binance spot (USDT-quoted) is only a proxy.

### Claims

- Since 7 Aug 2026, crypto Up/Down markets (5m, 15m, 4h) resolve on Chainlink-computed TWAPs instead of a single price snapshot. Both the price to beat and the final price come from the same TWAP feed. **[crucial]**  
  _Confidence: high; source date: 2026-08-07 (changelog); resolution page current as of 2026-10-26_  
  Sources: <https://docs.polymarket.com/changelog/predictions>, <https://docs.polymarket.com/concepts/resolution.md>  

- 5-minute markets used a 30-second TWAP lookback from 7 Aug 2026 and moved to a 60-second lookback on 14 Aug 2026 at 00:00 UTC. Live market config today shows cryptoMarketConfig {id: btc-5m-twap-60, twapLookbackSeconds: 60}. 15m markets show btc-15m-twap-60. **[crucial]**  
  _Confidence: high; source date: 2026-08-14; live API checked 2026-10-26_  
  Sources: <https://docs.polymarket.com/changelog/predictions>, <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793026800>  

- Resolution rule text: 'Up' if the Chainlink TWAP for the time range is greater than or equal to the price at the beginning of the range, otherwise 'Down'. An exact tie resolves UP. The resolution source is https://data.chain.link/streams/btc-usd-twap-60s-streams, and the rules say the market is about the Chainlink TWAP stream, 'not according to any other sources or spot markets'. **[crucial]**  
  _Confidence: high; source date: 2026-10-26 (live market)_  
  Sources: <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793026800>, <https://docs.polymarket.com/concepts/resolution.md>  

- Empirically, the price to beat of window N equals the final TWAP price of window N-1 exactly (Gamma eventMetadata.priceToBeat vs finalPrice across 15 consecutive BTC windows). This fits the docs: the price to beat is the 60s TWAP at window start, i.e. the average of the last 60s of the previous window. One third-party post claiming the open is still an instantaneous snapshot conflicts with this and appears outdated or wrong. **[crucial]**  
  _Confidence: high; source date: 2026-10-26 (own API sampling); tradoxvps updated 2026-10-09; casatrick 2026-08-11_  
  Sources: <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793025900>, <https://docs.polymarket.com/concepts/resolution.md>, <https://tradoxvps.com/polymarket-twap-settlement/>, <https://casatrick.substack.com/p/polymarket-twap-half-fixed>  

- Because the final price is the TWAP of the last 60 seconds of the 5-minute window, at T-30s roughly half of the averaging window is already observed. The edge model must estimate the remaining part of the average, not compare instantaneous spot with the start price. Third-party analysis reports that last-second sniping stopped working after the TWAP switch. **[crucial]**  
  _Confidence: high; source date: 2026-08 to 2026-10_  
  Sources: <https://docs.polymarket.com/market-data/realtime-data.md>, <https://tradoxvps.com/polymarket-twap-settlement/>  

- Chainlink has not published the exact TWAP computation (sampling, weighting, rounding, missing data), so a locally reproduced TWAP is only an approximation. Polymarket streams the official 60s TWAP value.  
  _Confidence: medium; source date: 2026-10-09_  
  Sources: <https://tradoxvps.com/polymarket-twap-settlement/>, <https://docs.polymarket.com/market-data/realtime-data.md>  

- 5-minute Up/Down markets currently exist for 8 assets: BTC, ETH, SOL, XRP, DOGE, HYPE, BNB, ZEC, each with a matching <asset>-usd-twap-60s-streams resolution source. All 8 also have 15m and 4h markets.  
  _Confidence: high; source date: 2026-10-26 (live API probe)_  
  Sources: <https://gamma-api.polymarket.com/events/slug/eth-updown-5m-1793026800>, <https://docs.polymarket.com/api-reference/live-data/overview.md>  

- Liquidity is concentrated in BTC. 24h series volume on 2026-10-26: BTC 5m ~$12.3M, ETH ~$314k, SOL ~$221k, XRP ~$26k, DOGE ~$12k, BNB ~$5k, HYPE ~$1.4k, ZEC ~$30. Individual BTC 5m windows traded roughly $33k-$100k each in the sample. **[crucial]**  
  _Confidence: medium; source date: 2026-10-26 (live API)_  
  Sources: <https://gamma-api.polymarket.com/series?slug=btc-up-or-down-5m>, <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793025600>  

- Order book snapshot at T-56s (one sample only). BTC: favorite-side top level ~1,239 shares at 0.67 with a 1-cent spread, and about $150-$700 within 3 cents on each side. ETH: thinner, with 4-cent spread. SOL: almost empty (a few shares at top). Expect only a few hundred to a few thousand dollars of fillable size per BTC window near the favorite price, and much less on other assets. **[crucial]**  
  _Confidence: low; source date: 2026-10-26 (single snapshot)_  
  Sources: <https://clob.polymarket.com/book?token_id=<live token>>  

- Slug naming is deterministic: {asset}-updown-5m-{unix_start_seconds}, with start aligned to multiples of 300s (e.g. btc-updown-5m-1793026800 = 15:00-15:05 UTC). 15m uses {asset}-updown-15m-{start aligned to 900}, and 4h uses {asset}-updown-4h-{start aligned to 14400}. Event title format: 'Bitcoin Up or Down - October 26, 11:00AM-11:05AM ET'.  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793026800>, <https://gamma-api.polymarket.com/events/slug/btc-updown-15m-1793026800>  

- 5-minute windows are created and start accepting orders about 24 hours before the window starts (createdAt ~23.96h before endDate; slugs exist out to +23h and return 404 at +24h). Each event carries startTime/eventStartTime (window open) and endDate (window close). The market has two outcomes [Up, Down] with two clobTokenIds and a conditionId.  
  _Confidence: high; source date: 2026-10-26 (live API)_  
  Sources: <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793026800>  

- Discovery: GET https://gamma-api.polymarket.com/events/slug/<slug> returns the event, markets, clobTokenIds, feeSchedule and cryptoMarketConfig. Series IDs: BTC 5m = 10684 (btc-up-or-down-5m), BTC 15m = 10192, BTC hourly = 10114, BTC 4h = 10331. Listing events by series_id with closed=false returns stale, never-closed events from Dec 2025, so computing slugs from the clock is more reliable.  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://gamma-api.polymarket.com/series?slug=btc-up-or-down-5m>, <https://gamma-api.polymarket.com/events?series_id=10684&closed=false>, <https://docs.polymarket.com/market-data/discover-markets.md>  

- Hourly crypto Up/Down markets do NOT use Chainlink TWAP. They resolve on the Binance BTC/USDT 1-hour candle (close >= open = Up). Slug pattern is e.g. bitcoin-up-or-down-october-26-2026-11am-et. Daily markets reportedly use Binance candles too.  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://gamma-api.polymarket.com/events/slug/bitcoin-up-or-down-october-26-2026-11am-et>, <https://tradoxvps.com/polymarket-twap-settlement/>  

- TWAP Up/Down markets resolve automatically without UMA proposal or dispute. Gamma shows automaticallyResolved=true and fallbackResolutionType=none. Data API /v2/resolutions shows status=resolved, was_disputed=false, payouts [1000000,0] and an on-chain tx about 80s after window end.  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://docs.polymarket.com/concepts/resolution.md>, <https://data-api.polymarket.com/v2/resolutions?condition=0xede58d311de130a0666dd8e289dc967dfb5591803360248db4d8fbc9b7df850d>  

- Resolution latency: 15 sampled BTC 5m windows had closedTime 78-80s after endDate. One window was still unresolved more than 133s after end. An earlier Bitquery study (data to Aug 2026) reported a median of 26s and a 90th percentile of 85s. Plan for 1.5-3+ minutes before capital is redeemable. **[crucial]**  
  _Confidence: medium; source date: 2026-10-26 (own sampling); Bitquery verified 2026-08-24_  
  Sources: <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793025900>, <https://bitquery.io/investigations/polymarket-oracle-race>  

- There is no trading cutoff before window end. The market keeps accepting orders AFTER endDate until resolution: at end+51s, acceptingOrders=true and closed=false, trading at 0.975/0.025. Trading stops once the market resolves. Bitquery documented large post-close 'gap' trading, mostly in Apr-May 2026. **[crucial]**  
  _Confidence: medium; source date: 2026-10-26 (live check)_  
  Sources: <https://clob.polymarket.com/markets/<conditionId>>, <https://docs.polymarket.com/concepts/resolution.md>, <https://bitquery.io/investigations/polymarket-oracle-race>  

- Winning tokens are not auto-converted to cash via the API. They must be redeemed with redeemPositions (gasless via Relayer with a Relayer or Builder API key) for 1 pUSD each, with no redemption deadline. A bot running many small bets needs an automated redeem loop, or capital stays locked in winning tokens. The web UI may offer auto-redeem, but I could not verify it from official docs. **[crucial]**  
  _Confidence: medium; source date: 2026 (docs current); third-party auto-redeem mention undated_  
  Sources: <https://docs.polymarket.com/trading/positions/manage.md>, <https://docs.polymarket.com/concepts/resolution.md>, <https://startpolymarket.com/hi/sikhe/how-markets-resolve/>  

- Collateral is pUSD (a USDC-backed ERC-20 on Polygon) since the CLOB V2 cutover on 28 Apr 2026. Orders must use V2 signing (official Rust client rs-clob-client-v2 exists). Fees are set at match time, not in the order. **[crucial]**  
  _Confidence: high; source date: 2026-04-28_  
  Sources: <https://docs.polymarket.com/changelog/predictions>, <https://docs.polymarket.com/concepts/pusd.md>  

- Crypto fees: taker fee = shares × 0.07 × p × (1-p), with makers paying zero and receiving a 20% maker rebate. Live market feeSchedule: {rate: 0.07, exponent: 1, takerOnly: true, rebateRate: 0.2}, feeType crypto_fees_v2. At p=0.92 the fee is ~0.00515 USDC/share (~0.56% of cost), so breakeven win rate is ~92.5%. At p=0.95 the fee is ~0.35%. The Feb 2026 launch note citing 1.56% peak at 50% is outdated (now 1.75%). **[crucial]**  
  _Confidence: high; source date: 2026-03-30 (Fee Structure V2); live 2026-10-26_  
  Sources: <https://docs.polymarket.com/trading/fees.md>, <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793026800>, <https://docs.polymarket.com/changelog/predictions>  

- Marketable (taker) orders on crypto Up/Down markets are held by a 'taker delay' of 150ms (since 4 Sep 2026; it was 250ms, then 50ms from 17 Aug) and cannot be cancelled during the hold. Checks re-run after the delay. The order-lifecycle doc page still says 250ms, so it is stale. **[crucial]**  
  _Confidence: high; source date: 2026-09-04_  
  Sources: <https://docs.polymarket.com/changelog/predictions>, <https://docs.polymarket.com/concepts/order-lifecycle.md>  

- Minimum order size is 5 shares. Tick size is 0.01 normally (live market). Resolved or extreme-priced markets showed a 0.001 tick, so tick size can change: listen for tick_size_change events.  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793026800>, <https://docs.polymarket.com/trading/place-orders.md>  

- RTDS (crypto_prices = Binance, crypto_prices_chainlink, crypto_prices_twap_*) has been LEGACY for reference prices since 15 Sep 2026, with planned removal about one month after SDK 0.11.0. The replacement is the PolyBolt WebSocket wss://ws-live-v2.polymarket.com/ws with channels price.crypto (Chainlink by default since 2 Oct 2026, optional Pyth pin) and price.crypto.twap (Chainlink 60s TWAP, window_seconds must be 60). Both REQUIRE CLOB API credentials. The 30s TWAP topic has no replacement. **[crucial]**  
  _Confidence: high; source date: 2026-09-15 / 2026-10-02_  
  Sources: <https://docs.polymarket.com/migrate/rtds-to-polybolt.md>, <https://docs.polymarket.com/api-reference/live-data/overview.md>, <https://docs.polymarket.com/changelog/predictions>  

- PolyBolt limits: 64 subscriptions per connection, 20 subscribe/unsubscribe frames per second, 8 auth frames per connection, server ping every 25s (two missed pongs close the connection with 4002). Each subscribe returns a 2-minute snapshot. Re-auth and re-subscribe after reconnect, and re-read the 'source' field.  
  _Confidence: high; source date: 2026-10_  
  Sources: <https://docs.polymarket.com/api-reference/live-data/overview.md>  

- Direct Chainlink Data Streams access (the actual resolution stream with signed reports) requires a paid subscription with no free tier, via app.chain.link. Polymarket's PolyBolt TWAP stream is the practical free source, but it is a relay, and decoded reports are not signature-verified by the SDK.  
  _Confidence: medium; source date: 2026_  
  Sources: <https://docs.chain.link/data-streams/sign-up>, <https://tradoxvps.com/polymarket-twap-settlement/>  

- Binance spot differs from the resolution source in three ways. Venue: Chainlink aggregates multiple unnamed venues. Quote currency: USDT vs USD. Method: settlement uses a 60s TWAP, while Binance gives instantaneous trades. Binance should only be a leading indicator or proxy, with basis and lag calibrated against the PolyBolt Chainlink price and TWAP. **[crucial]**  
  _Confidence: medium; source date: 2026-09 / 2026-10_  
  Sources: <https://docs.polymarket.com/migrate/rtds-to-polybolt.md>, <https://tradoxvps.com/polymarket-twap-settlement/>  

- Gamma eventMetadata.priceToBeat for a window did NOT appear until well after that window started (still null at 133s into the window, apparently filled when the previous window resolved). The bot should take the price to beat from its own TWAP stream at the window-start timestamp, not from Gamma. **[crucial]**  
  _Confidence: medium; source date: 2026-10-26 (own observation, few samples)_  
  Sources: <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793027700>  

- The matching engine restarts periodically for maintenance, with about 2 days' notice on Telegram/Discord. After a restart it runs temporary post-only or cancel-only modes and returns Retry-After. The bot must handle these modes. Released CLOB order rate limits are high (e.g. POST /order 200/s sustained since 1 Jun 2026).  
  _Confidence: high; source date: 2026-06 to 2026-10_  
  Sources: <https://docs.polymarket.com/trading/matching-engine.md>, <https://docs.polymarket.com/changelog/predictions>  

- Gamma marks these events/markets as restricted=true. Polymarket's geoblock endpoint (polymarket.com/api/geoblock) returned blocked=false for a Serbian (RS) IP during this research. Re-check from the actual home server and any cloud VPS region, because restricted markets are geofenced per country. **[crucial]**  
  _Confidence: medium; source date: 2026-10-26_  
  Sources: <https://polymarket.com/api/geoblock>, <https://gamma-api.polymarket.com/events/slug/btc-updown-5m-1793026800>  

### Risks

- The strategy design assumes 'live price vs window start price'. Actual settlement is a 60s Chainlink TWAP compared with a TWAP price to beat. A model built on Binance spot will be mis-calibrated, especially near the strike.
- Thin books: outside BTC (and partly ETH/SOL), depth at 92-99c is a few shares to a few hundred dollars. High-volume small bets will move prices or go unfilled, and paper trading that assumes fills at the displayed price will overstate PnL. The paper engine must simulate book depth, the 150ms taker delay and fees.
- Fee drag: at 0.92, the taker fee (~0.56%) lifts breakeven win probability to ~92.5%. The edge per trade on a 92c contract is tiny, and one loss wipes out about 11 wins. Using maker (post-only) orders avoids fees and earns rebates but carries fill risk.
- Capital lock-up: winnings need on-chain redemption (gasless Relayer). Resolution usually takes ~80s but can exceed 2+ minutes. With many concurrent bets, an automated redeem loop is needed or available balance shrinks.
- Post-close trading: markets keep trading after window end until resolution. Faster participants who already know the final TWAP can pick off stale resting orders, so cancel resting orders at window end.
- API churn: RTDS price topics are deprecated, with removal about one month after SDK 0.11.0, and must not be built on. Fee, taker-delay and TWAP-window parameters changed several times in 2026 (30s→60s TWAP; 250→50→150ms delay), and doc pages lag the changelog. Read feeSchedule and cryptoMarketConfig from each market at runtime.
- PolyBolt reference prices need CLOB API credentials even for paper trading. Phase 1 still needs a Polymarket account and API key derived from the wallet.
- Geoblocking: markets are flagged restricted. A cloud VPS in a blocked jurisdiction (e.g. US, some EU countries) could be refused even if Serbia itself is allowed. Region choice for the VPS matters.
- Third-party sources (tradoxvps, casatrick, Bitquery) have commercial motives or conflicting claims. Casatrick's claim that the open is a snapshot conflicts with official docs and my own data.

### Open questions

- What is Chainlink's exact TWAP sampling method (1s samples? mid vs benchmark price? handling of missing data)? It is not published, so how closely can a local estimate track it?
- What exactly decides when resolution happens after window end (typically ~80s, sometimes >2 min), and is there a fallback if the Chainlink TWAP stream is stale or unavailable (fallbackResolutionType=none)?
- Is there an account-level auto-redeem for API/Deposit Wallet users, or must the bot always call redeemPositions? Does the UI 'auto-redeem' apply to wallets driven by an API key?
- What are order book depth and spread in the final 60s across many windows and times of day? Only one snapshot was taken, because automated polling with a spoofed user agent was blocked in this session. A proper sampler using the official SDK or WebSocket market channel is needed.
- Do the 92-99c price levels in the last 30-45s actually offer fillable size after the 150ms taker delay, or are they mostly quoted by fast market makers who pull quotes on Binance moves?
- Is the PolyBolt TWAP value identical to the value used for settlement (eventMetadata.finalPrice)? This should be checked by logging both over many windows.
- Which cloud VPS regions are geoblocked for restricted markets? Is trading from Serbia definitively allowed under Polymarket's ToS (not just the geoblock endpoint)?
- Are 5m markets eligible for current liquidity rewards or maker rebates, which could make a maker strategy better than taking at 92c+?

---

## 2. Fees and economics of Polymarket 5-minute and 15-minute crypto Up/Down markets (as of 2026-10-26)

### Summary

Fees are live and confirmed. I checked them on the live API on 2026-10-26, not only in the docs. Every BTC/ETH/SOL 5m market and the BTC 15m market I queried returns feeType "crypto_fees_v2" with feeSchedule {rate 0.07, exponent 1, takerOnly true, rebateRate 0.2}. They also show minimum order size 5 shares and a normal tick of 0.01. Right after a window closed, the tick switched to 0.001 (best bid 0.999).

- **Fee formula:** taker fee = shares × 0.07 × p × (1−p), charged at match time. Makers pay nothing.
- **Fee per share at each price:** 0.92 → $0.00515; 0.95 → $0.00333; 0.97 → $0.00204; 0.99 → $0.00069.
- **Fee as a share of a win:** at any price the fee takes about 6.4–6.9% of a winning trade's profit.
- **Break-even win rates including fees:** 92.52%, 95.33%, 97.20% and 99.07% respectively.
- **Wins needed to cover one loss:** 12.4, 20.4, 34.8 and 106.4 respectively.
- **The bigger cost is the spread, not the fee.** If the market shows 0.92 but you have to buy at the 0.93 ask, the break-even rises about one point to roughly 93.5%. That one cent is 12.5% of the maximum win.

Other costs and rebates:
- **Maker rebates:** 20% of crypto taker fees go to makers, paid daily.
- **Taker rebates:** a tiered program has existed since 2026-05-28, ranging from 3% to 50% of fees, based on 30-day weighted volume = size × (1−p) × 2.3. Buying near 0.92–0.99 builds weighted volume very slowly.
- **Gas:** the relayer pays gas for proxy, Safe and Deposit wallets, including redeem. A raw EOA wallet pays its own gas in POL.
- **Polymarket itself:** no deposit or withdrawal fee. Card onramp (MoonPay) is roughly 2–4%. The offramp is about 3–5% and its availability for Serbia is unconfirmed.

Changes during 2026 that affect the bot:
- CLOB V2 went live 2026-04-28. Orders now have to be signed the V2 way, and pUSD replaced USDC.e as collateral.
- A taker delay is on (live flag itode=true). The docs say 250 ms; a third-party post says 150 ms. During the delay an order cannot be cancelled.
- Most important: the markets now settle on a Chainlink 60-second TWAP (live: twapLookbackSeconds 60), not a single spot tick. Point-price models built from Binance spot will be miscalibrated unless they model the TWAP.

Why volume alone gives no edge: each trade's expected value is (true probability − price − fee). If that number is zero or negative, more trades just shrink the variance around a negative or zero result. Bitquery found the median result got worse with more markets played: about −$102 for wallets that played more than 500 markets. Paper trading must model the ask you actually pay, the fee, the taker delay and TWAP settlement, or its results will be misleading.

### Claims

- Taker fee on crypto markets (incl. 5m and 15m Up/Down) = C × feeRate × p × (1−p) with feeRate 0.07 (exponent 1); makers pay 0; fees are computed in pUSD/USDC at match time, rounded to 5 decimals (min 0.00001). Verified live on 2026-10-26: btc/eth/sol-updown-5m and btc-updown-15m all return feeType crypto_fees_v2, feeSchedule {exponent:1, rate:0.07, takerOnly:true, rebateRate:0.2}; clob-markets endpoint returns fd {r:0.07,e:1,to:true}. **[crucial]**  
  _Confidence: high; source date: 2026-10-26 (live API) / docs current_  
  Sources: <https://docs.polymarket.com/trading/fees>, <https://gamma-api.polymarket.com/events?slug=btc-updown-5m-1793026800>, <https://clob.polymarket.com/clob-markets/0x447eb28cc3b6222d3767825451c8af6efd806d0eeacc99de89cadfab2faad96a>, <https://docs.polymarket.com/v2-migration>  

- Fee per share / break-even win rate / wins needed to offset one loss (taker, buying at shown price, holding to resolution): p=0.92: $0.00515, 92.52%, 12.4; p=0.95: $0.00333, 95.33%, 20.4; p=0.97: $0.00204, 97.20%, 34.8; p=0.99: $0.00069, 99.07%, 106.4. Fee is 0.07×(1−p) of notional (0.56%/0.35%/0.21%/0.07%) but ~6.4–6.9% of each win's profit at all these prices. **[crucial]**  
  _Confidence: high; source date: computed 2026-10-26 from live fee params_  
  Sources: <https://docs.polymarket.com/trading/fees>  

- Fee history: taker fees began on 15-minute crypto markets ~January 5, 2026 (earlier formula: feeRate 0.25, exponent 2, peaking about 1.56% at p=0.5), expanded to all crypto timeframes by ~March 6, and Fee Structure V2 (crypto 0.07, exponent 1) took effect ~March 30, 2026. Bitquery says the effective rate peaked ~4.2% in March and fell to ~1.1% in May after the new exchange contract.  
  _Confidence: medium; source date: Jan–Aug 2026_  
  Sources: <https://pineanalytics.substack.com/p/polymarket-fee-rollout>, <https://cointelegraph.com/news/polymarket-quietly-adds-taker-fees-15-minute-crypto-markets>, <https://www.theblock.co/post/384461/polymarket-adds-taker-fees-to-15-minute-crypto-markets-to-fund-liquidity-rebates>, <https://www.sacra.com/c/polymarket/>, <https://www.bitquery.io/investigations/polymarket-five-minute-casino>  

- Maker Rebates Program: 20% of crypto taker fees are rebated to makers, split per market by each maker's fee_equivalent share, paid daily in pUSD once accrued ≥$1; queryable via GET https://clob.polymarket.com/rebates/current. Polymarket can change the percentages. The live markets also show makerRebatesFeeShareBps 10000 alongside rebateRate 0.2, and how the two fields relate is unclear.  
  _Confidence: high; source date: 2026 (docs current)_  
  Sources: <https://docs.polymarket.com/polymarket-learn/trading/maker-rebates-program>, <https://gamma-api.polymarket.com/events?slug=btc-updown-5m-1793026800>  

- Taker Rebate Program (launched 2026-05-28): tiers by 30-day Weighted Volume = trade size × (1 − entry price) × category weight (Crypto 2.3). Bronze $2k wV = 3%, Silver $20k = 8%, Gold $200k = 18%, Platinum $1M = 32%, Diamond $4M = 44%, Obsidian $10M+ = 50%; paid daily, no backfill. Buying at 0.92 earns only 0.184 wV per $1 traded, so Bronze needs ~$10.9k of notional per 30 days, and at 0.99 ~$87k.  
  _Confidence: high; source date: 2026-05 (docs current)_  
  Sources: <https://docs.polymarket.com/programs/taker-rebates>  

- Liquidity rewards config is present on 5m markets (rewardsMinSize 50 shares, rewardsMaxSpread 4.5c), but the dedicated $1M August crypto TWAP rewards campaign is marked ended. Outside a midpoint of 0.10–0.90, quotes must be two-sided to score, so near-0.92+ one-sided bids earn nothing.  
  _Confidence: medium; source date: 2026-10-26 live / docs current_  
  Sources: <https://docs.polymarket.com/market-makers/liquidity-rewards>, <https://clob.polymarket.com/clob-markets/0x447eb28cc3b6222d3767825451c8af6efd806d0eeacc99de89cadfab2faad96a>  

- Minimum order size on 5m/15m crypto markets is 5 shares, so a minimum bet at 0.92 costs about $4.63 including fee. Normal tick size is 0.01. After the 15:00–15:05 UTC window closed on 2026-10-26, the book showed tick_size 0.001 with best bid 0.999, so the tick does shrink near the extremes. The exact trigger price (commonly cited as >0.96/<0.04) is unverified. Listen for the tick_size_change websocket event and never hardcode the tick.  
  _Confidence: medium; source date: 2026-10-26 (live observation)_  
  Sources: <https://clob.polymarket.com/book?token_id=10517842848853839548883881760807504734225708723202456143219597770342107445648>, <https://docs.polymarket.com/market-data/realtime-data.md>, <https://docs.polymarket.com/api-reference/market-data/get-tick-size.md>  

- Taker order delay is active on crypto up/down markets (live clob-markets returns itode:true). Official docs say the hold is 250 ms, during which the order cannot be cancelled and is re-validated before matching. A third-party post claims the crypto delay was raised from 50 ms to 150 ms on 2026-09-04, so the exact current value is uncertain. (The original 500 ms delay was removed ~Feb 18, 2026.) **[crucial]**  
  _Confidence: medium; source date: Feb–Sep 2026, live 2026-10-26_  
  Sources: <https://docs.polymarket.com/concepts/order-lifecycle>, <https://clob.polymarket.com/clob-markets/0x447eb28cc3b6222d3767825451c8af6efd806d0eeacc99de89cadfab2faad96a>, <https://dev.to/abrownfox001/what-actually-changed-for-bots-63l>, <https://protos.com/polymarket-ends-trading-loophole-for-bitcoin-quants/>  

- 5m/15m crypto markets now resolve on a Chainlink TWAP, not a single end-of-window tick. Live rules text: Up if the Chainlink BTC/USD TWAP for the window is ≥ the price at the start of the window, using the btc-usd-twap-60s data stream. The live config shows twapEnabled true, twapLookbackSeconds 60. TWAP settlement started ~2026-08-07 (initially 30 s for 5m markets) and moved to 60 s ~2026-08-14. In the last 30–45 s, much of the settling average is already fixed, and Binance spot is not the settlement source. **[crucial]**  
  _Confidence: high; source date: Aug 2026; live 2026-10-26_  
  Sources: <https://gamma-api.polymarket.com/events?slug=btc-updown-5m-1793026800>, <https://data.chain.link/streams/btc-usd-twap-60s-streams>, <https://cryptobriefing.com/polymarket-twap-upgrade-liquidity-rewards/>, <https://dev.to/abrownfox001/what-actually-changed-for-bots-63l>  

- CLOB V2 went live 2026-04-28: V1 SDKs and V1-signed orders no longer work. The order struct changed (no nonce or feeRateBps; adds a millisecond timestamp, metadata and builder fields), and the EIP-712 exchange domain version is now "2". Collateral moved from USDC.e to pUSD; API-only traders must wrap USDC.e via the Collateral Onramp. Official SDKs are TypeScript (@polymarket/clob-client-v2) and Python (py-clob-client-v2), and no official Rust SDK is mentioned, so the Rust backend must implement V2 signing itself or use a community crate. **[crucial]**  
  _Confidence: high; source date: 2026-04-28_  
  Sources: <https://docs.polymarket.com/v2-migration>  

- Gas: the Polymarket relayer pays gas for Deposit, Safe and Proxy wallets, covering approvals, transfers, split/merge/redeem. Order matching is submitted on-chain by the operator, so the trader pays no gas per trade. A plain EOA pays all on-chain actions in POL. Relayer or Builder API keys are needed for gasless operations via the API. There is no fee on redeem or settlement beyond the taker fee.  
  _Confidence: medium; source date: 2026 (docs current)_  
  Sources: <https://docs.polymarket.com/trading/gasless>, <https://docs.polymarket.com/concepts/order-lifecycle>, <https://docs.polymarket.com/trading/wallets-auth>  

- Polymarket charges no deposit or withdrawal fee, though intermediaries such as MoonPay or Coinbase charge their own. A withdrawal unwraps pUSD to USDC via the Collateral Offramp and swaps through a Uniswap v3 pool (UI requires under 10 bp output difference; the pool can run short of liquidity). Amounts over $50k should be split.  
  _Confidence: high; source date: 2026 (docs current)_  
  Sources: <https://docs.polymarket.com/trading/fees>, <https://docs.polymarket.com/trading/bridge/withdraw>  

- Card deposit via MoonPay (built into the deposit screen) costs roughly 2–4% all-in (card plus FX spread), with about a $20 minimum and separate MoonPay KYC. Selling to bank or card via MoonPay costs roughly 3–5%. These figures come from third-party guides, not official sources.  
  _Confidence: low; source date: ~Apr–May 2026_  
  Sources: <https://pm.wiki/sv/learn/polymarket-deposit-guide>, <https://pm.wiki/ar/learn/polymarket-withdrawal-guide>, <https://startpolymarket.com/guides/how-to-deposit/>  

- Bank withdrawal from Serbia is uncertain. MoonPay's sell payouts by SEPA are listed for the EUR region only, with Visa card payouts for the rest of the world. Serbia is not on MoonPay's official unsupported list, but an Exodus help article reportedly lists Serbia as a country where selling via MoonPay is unavailable (I could not open that page). Serbia joined SEPA's geographic scope on 2025-05-22, with Serbian banks operationally ready from about May 2026. A fallback is to withdraw USDC to an exchange that serves Serbia and sell there. **[crucial]**  
  _Confidence: low; source date: 2025-05 to 2026_  
  Sources: <https://support.moonpay.com/customers/docs/supported-bank-accounts-for-withdrawals>, <https://support.moonpay.com/customers/docs/moonpays-unsupported-countries>, <https://www.exodus.com/support/article/2435-sell-crypto-moonpay>, <https://www.europeanpaymentscouncil.eu/sites/default/files/kb/file/2025-05/EPC172-25%20Press%20Release%20Serbia_0.pdf>  

- Serbia (RS) is not on Polymarket's blocked or close-only geoblock list. The check is GET https://polymarket.com/api/geoblock. Candidate cloud VPS locations matter: US, UK, DE, FR, IT, PL, SG, AU and others are close-only via the API, while IE, NL and JP are frontend-only (API not restricted). **[crucial]**  
  _Confidence: medium; source date: docs current (undated)_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock>  

- Serbian individuals pay 15% capital-gains tax on profits from disposing of digital assets (Law on Personal Income Tax / Law on Digital Assets), and losses can offset gains. How Polymarket outcome-token winnings are classified under Serbian law was not established.  
  _Confidence: low; source date: 2026 (law-firm blogs)_  
  Sources: <https://zuniclaw.com/en/cryptocurrency-and-digital-asset-tax-in-serbia/>, <https://statt.rs/tax-aspect-of-cryptocurrency-trading-in-serbia/>  

- Volume does not create edge. Per-trade EV = q − (p + fee), where q is the true probability; doing N trades multiplies EV by N and only shrinks variance relative to it. On-chain analysis of 5m markets found the crowd roughly break-even before fees and losing about the full fee after; fewer than a third of wallets were ahead in a sampled week, and median results worsened with activity (about −$0.96 for one-market wallets vs −$101.61 for wallets that played over 500 markets). **[crucial]**  
  _Confidence: medium; source date: Sep 2025–Aug 2026 data_  
  Sources: <https://www.bitquery.io/investigations/polymarket-five-minute-casino>  

### Risks

- The spread costs more than the fee. The fee at 0.92 is about 0.5 cents per share, but crossing a 1-cent spread (market shows 0.92, ask is 0.93) costs 1 cent and pushes the break-even win rate to about 93.5%. Late in the window the book thins out (in one observed snapshot the top asks were under 60 shares), so larger orders walk the book.
- TWAP settlement (60 s Chainlink TWAP) breaks naive 'spot vs start price' models. The bot must rebuild the Chainlink TWAP and model the part of the 60 s average that is still unknown. Binance spot can diverge from the Chainlink stream.
- Adverse selection. A 0.92+ price in the last 30–45 s is usually already efficient, because colocated bots see the same feeds. The 250 ms (or 150 ms) taker delay, during which an order cannot be cancelled, plus home-server latency from Serbia means fills tend to happen exactly when the price moves against you.
- Tail-loss asymmetry. At 0.97, one loss wipes out about 35 wins; at 0.99 about 106. A small calibration error (true probability 1–2 points below the estimate) turns the strategy negative, and a few hundred trades are not enough to measure a win rate to 1-point precision.
- Fees and rules change fast. Fee formulas changed in January, March (V2) and May (new exchange contract) of 2026, and taker-delay values changed several times. Read feeSchedule, fd, mts, mos and itode from the API on every market rather than hardcoding them.
- CLOB V2 signing has no official Rust SDK. A wrong EIP-712 domain or order struct means rejected orders, and holding the wallet private key on a home server is a custody risk.
- The fiat offramp to a Serbian bank may not work through MoonPay. Users could be stuck with USDC and need a crypto exchange that serves Serbia, which is the opposite of the 'no crypto wallet handling' requirement.
- Cloud VPS geography: hosting in the US, UK, DE, FR, PL, SG or another listed country would put the API in close-only mode. Choose a region whose IP country is not restricted, and check the geoblock endpoint from the server.
- Polymarket can change the taker rebate and maker rebate percentages at any time without notice. Do not count on rebates in the economics.

### Open questions

- What is the exact current taker delay on crypto 5m markets: 250 ms (docs) or 150 ms (third-party report from Sep 2026)? It can be measured empirically once live.
- At what price does the tick size switch from 0.01 to 0.001 on 5m markets (commonly cited as >0.96/<0.04, unverified)? Also, is the fee charged in pUSD on top of the order or deducted from shares received, for limit and for market buys?
- Does MoonPay (or another Polymarket-integrated offramp) actually support selling to a Serbian bank account or card in RSD/EUR? If not, which exchange serving Serbia should be the USDC-to-fiat route, and what are its fees?
- How does the settlement TWAP define the 'price at the beginning' of the window: a TWAP value at the start, or a spot tick? And is the 60 s lookback the last 60 s of the window?
- What is the meaning of makerRebatesFeeShareBps=10000 versus rebateRate=0.2 in the live market config?
- Do 5m crypto taker trades count toward the Taker Rebate Program? Docs say only fee-paying taker trades count and crypto is weighted 2.3, but 5m markets are not named explicitly.
- How should Polymarket winnings be treated under Serbian tax law (digital-asset capital gain vs gambling/other income), and are there any Serbian games-of-chance licensing issues for a resident using an offshore prediction market?
- Which wallet type will the user's card-funded account have (Magic/email proxy, Safe, or the new Deposit Wallet)? That determines the signatureType and the relayer flow the Rust bot must implement.

---

## 3. Technical API stack for a Rust Polymarket 5-minute crypto Up/Down bot (CLOB REST/WS, Gamma, RTDS/PolyBolt, auth, Rust SDK, rate limits, latency), verified 2026-10-26

### Summary

Verified 2026-10-26 against docs.polymarket.com, GitHub and live API calls made from Serbia. Several of the project's assumptions are out of date.

(1) Settlement price. The 5m markets no longer settle on a spot price. The live Gamma record for btc-updown-5m-<startUnix> shows cryptoMarketConfig {twapEnabled:true, twapLookbackSeconds:60} and the resolution source "btc-usd-twap-60s-streams". The outcome compares two readings of the same Chainlink 60s TWAP stream: the start value and the end value. So in the last 30-45s, half or more of the final averaging window is already fixed. Model the Chainlink TWAP, not Binance spot. In a live sample, Binance BTCUSDT was about $200 (0.26%) above Chainlink BTC/USD.

(2) Taker delay. On these markets marketable orders are held for 250 ms and then re-validated (clob-markets field itode=true). An order cannot be cancelled while it is held.

(3) Fees. Taker fee is 0.07 × shares × p × (1-p); makers pay nothing. At p=0.92 that is about 0.56% of stake.

(4) Price feeds. Legacy RTDS (wss://ws-live-data.polymarket.com) price topics are deprecated. Their removal was planned for about one month after SDK 0.11.0 (released 2026-09-23), but they still streamed on 2026-10-26. The replacement, PolyBolt (wss://ws-live-v2.polymarket.com/ws, channels price.crypto and price.crypto.twap), requires CLOB API credentials. That means paper trading needs a Polymarket account and API key too.

(5) Wallets. Accounts created on or after 2026-05-04 get a Deposit Wallet: signatureType 3, called Poly1271 in the Rust SDK, with the deposit wallet as funder. Email/Google accounts created earlier use POLY_PROXY (type 1). Email-account keys are exported at reveal.magic.link/polymarket. Session Keys (beta) can trade but cannot withdraw, so they are safer than the owner key. They need a Builder API key and manual approval by email to builder@polymarket.com, and the Rust SDK does not support them. EOA (type 0) trading requires the address to be allowlisted.

(6) Rust SDK. The official crate is polymarket_client_sdk_v2 v0.8.0 (2026-09-10), repo Polymarket/rs-clob-client-v2. The v1 rs-clob-client is archived. It has clob, ws, gamma, data, rtds, heartbeats and ctf features but no PolyBolt and no session keys, 79 open issues including WebSocket resubscribe bugs, and its README points at clob-v2.polymarket.com, which does not resolve in DNS. GET clob.polymarket.com/version returns 2.

(7) Order cancel safety. POST /heartbeats every 5s. After about 10-15s without one, all orders for that API key are cancelled. Once armed it cannot be turned off.

(8) Rate limits. Order limits are per signer: Standard tier allows 40 orders/s with a burst of 60; excess requests get 429. Batches are capped at 15 orders and are all-or-nothing on rate limit.

(9) Matching engine restarts. These return 425, followed by 2 minutes where only post-only orders are accepted.

(10) Server location and geoblocking. Polymarket's servers are in AWS eu-west-2 (London), but the UK is close-only. DE, FR, PL, IT, BE and SK are also close-only, so many common EU VPS locations are blocked. eu-west-1 (Ireland) is the documented closest allowed region. Serbia (RS) is not restricted; the live geoblock check returned blocked:false, country RS.

(11) Redemption. Winning positions have to be redeemed. This can be done gas-free through relayer-v2 with a Relayer API key created in Settings.

### Claims

- BTC 5-minute Up/Down markets now resolve on Chainlink's 60-second TWAP stream, not spot. Gamma shows cryptoMarketConfig {id:'btc-5m-twap-60', twapEnabled:true, twapLookbackSeconds:60} and resolutionSource https://data.chain.link/streams/btc-usd-twap-60s-streams. The market resolves Up if the end TWAP is greater than or equal to the start reading ('price to beat') of the same TWAP stream. **[crucial]**  
  _Confidence: high; source date: 2026-10-26 (live API)_  
  Sources: <https://gamma-api.polymarket.com/markets?slug=btc-updown-5m-1793026800 (live call 2026-10-26)>, <https://docs.polymarket.com/concepts/resolution.md>  

- Slug pattern is btc-updown-5m-<unix start of window> (start = floor(now/300)*300). Markets belong to series 'btc-up-or-down-5m' and are created about 24h ahead. Gamma returns conditionId, clobTokenIds [Up, Down], eventStartTime, endDate, orderPriceMinTickSize (0.01), orderMinSize (5), negRisk (false), feeSchedule and version ('v1' = CTF token market).  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://gamma-api.polymarket.com/markets?slug=btc-updown-5m-1793026800>, <https://docs.polymarket.com/api-reference/markets/get-market-by-slug.md>  

- These markets apply a 250 ms taker-order delay (GET /clob-markets/{condition_id} returns itode:true). Marketable orders are held for 250 ms, then re-validated (balance, allowance, risk) and either matched or placed on the book. They cannot be cancelled while held. A submission can return status 'delayed'. **[crucial]**  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://clob.polymarket.com/clob-markets/0x447eb28cc3b6222d3767825451c8af6efd806d0eeacc99de89cadfab2faad96a (live)>, <https://docs.polymarket.com/concepts/order-lifecycle.md>, <https://docs.polymarket.com/api-reference/markets/get-clob-market-info.md>  

- Crypto market fees: feeType crypto_fees_v2, fee = shares × 0.07 × p × (1-p), taker-only, 20% maker rebate. GET /fee-rate returns base_fee 1000. V2 orders do not carry feeRateBps. **[crucial]**  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://docs.polymarket.com/trading/fees.md>, <https://gamma-api.polymarket.com/markets?slug=btc-updown-5m-1793026800>, <https://github.com/Polymarket/rs-clob-client-v2>  

- CLOB market WebSocket: wss://ws-subscriptions-clob.polymarket.com/ws/market. Subscribe with {assets_ids:[...], type:'market', custom_feature_enabled:true}. Events: book, price_change, last_trade_price, tick_size_change, plus best_bid_ask, new_market and market_resolved when custom features are on. Client must send text 'PING' every 10s. No auth.  
  _Confidence: high; source date: 2026 (current docs)_  
  Sources: <https://docs.polymarket.com/market-data/realtime-data.md>, <https://docs.polymarket.com/api-reference/wss/market.md>  

- CLOB user WebSocket: wss://ws-subscriptions-clob.polymarket.com/ws/user. Send {auth:{apiKey,secret,passphrase}, type:'user', markets:[conditionIds]} immediately after connecting. Order events: PLACEMENT/UPDATE/CANCELLATION. Trade statuses: MATCHED, MINED, RETRYING (non-terminal) and CONFIRMED, FAILED (terminal). PING every 10s. Missed events are not replayed, so reconcile over REST after reconnecting.  
  _Confidence: high; source date: 2026 (current docs)_  
  Sources: <https://docs.polymarket.com/trading/realtime-order-updates.md>  

- Legacy RTDS (wss://ws-live-data.polymarket.com, PING every 5s) price topics are deprecated: crypto_prices (Binance *usdt), crypto_prices_chainlink and crypto_prices_twap_sixty. Removal was planned one month after SDK 0.11.0 (npm release 2026-09-23). As of 2026-10-26 all three still streamed without auth, about 1 update/s per symbol. **[crucial]**  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://docs.polymarket.com/migrate/rtds-to-polybolt.md>, <https://registry.npmjs.org/@polymarket/client>, live websocket test 2026-10-26  

- PolyBolt (wss://ws-live-v2.polymarket.com/ws) replaces the RTDS price topics. Channels are price.crypto ({symbol:'btcusd'}, Chainlink by default) and price.crypto.twap (window_seconds 60). An auth frame with CLOB API credentials is required; an unauthenticated subscribe returned auth_required. The server pings every 25s. Limits: 64 subscriptions, close codes 4001/4002/4003/4008. Use full_accuracy_value (a decimal string). **[crucial]**  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://docs.polymarket.com/migrate/rtds-to-polybolt.md>, live websocket test 2026-10-26  

- Chainlink BTC/USD and Binance BTCUSDT differed by about $200 (~0.26%) in a live sample (80251 vs 80463). Using Binance as a direct stand-in for the settlement price is unsafe; use it only as a leading indicator. **[crucial]**  
  _Confidence: medium; source date: 2026-10-26_  
  Sources: live RTDS capture 2026-10-26 (crypto_prices vs crypto_prices_chainlink)  

- Auth: L1 = EIP-712 ClobAuth signature from the signer key (headers POLY_ADDRESS/SIGNATURE/TIMESTAMP/NONCE), used for POST /auth/api-key or GET /auth/derive-api-key. L2 = apiKey/secret/passphrase with HMAC-SHA256 over timestamp+method+path+body (headers POLY_ADDRESS, POLY_SIGNATURE, POLY_TIMESTAMP, POLY_API_KEY, POLY_PASSPHRASE).  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/trading/wallets-auth.md>  

- Signature types are 0 EOA, 1 POLY_PROXY (legacy Magic/email/Google wallets), 2 GNOSIS_SAFE (legacy browser-wallet) and 3 DEPOSIT_WALLET (EIP-1271; 'Poly1271' in the Rust SDK). All account wallets deployed on or after 2026-05-04 are Deposit Wallets. Funder/maker is the account wallet address shown in the profile menu. **[crucial]**  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/trading/wallets-auth.md>, <https://github.com/Polymarket/rs-clob-client-v2>  

- A new card-funded account created by email after May 2026 should trade with signature type 3 and funder = its deposit wallet. The owner key is presumably the Magic-held key. This is inferred: the docs do not explicitly tie Magic accounts to deposit-wallet ownership. **[crucial]**  
  _Confidence: medium; source date: 2026_  
  Sources: <https://docs.polymarket.com/trading/wallets-auth.md>, <https://help.polymarket.com/en/articles/13364258-how-do-i-export-my-key>  

- Email-account private key export: while logged in, open https://reveal.magic.link/polymarket, sign in to Magic and choose export. The article (2026-01-11) does not cover Google sign-in or deposit wallets. **[crucial]**  
  _Confidence: medium; source date: 2026-01-11_  
  Sources: <https://help.polymarket.com/en/articles/13364258-how-do-i-export-my-key>  

- Session Keys (beta) are a separate EOA authorized for CLOB trading on a Deposit Wallet for a fixed 180 days. They cannot withdraw funds. Creating one requires the owner key and a Builder API key approved via builder@polymarket.com. Not implemented in the Rust SDK (no session-signer code found). **[crucial]**  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/trading/session-keys.md>, <https://github.com/Polymarket/rs-clob-client-v2>  

- EOA (type 0) trading requires the EOA to be allowlisted and POL for gas.  
  _Confidence: medium; source date: 2026_  
  Sources: <https://docs.polymarket.com/trading/gasless.md>  

- Collateral is pUSD (6 decimals, USDC-backed wrapper). Before trading, the account wallet needs approvals for pUSD and outcome tokens (ERC-1155) on the exchange contracts. Deposit/Proxy/Safe wallets can set them gas-free via relayer-v2 (Relayer API key from Settings → API Keys); EOAs must approve on-chain themselves. Exchange addresses: CTF 0xE111180000d2663C0091e4f400237545B87B996B, NegRisk 0xe2222d279d744050d28e00520010520000310F59.  
  _Confidence: medium; source date: 2026_  
  Sources: <https://docs.polymarket.com/concepts/pusd.md>, <https://docs.polymarket.com/trading/gasless.md>, <https://docs.polymarket.com/trading/place-orders.md>, <https://github.com/Polymarket/rs-clob-client-v2>  

- Order types: GTC, GTD (expires 1 minute before the stated time and must be at least 3 minutes ahead, so it is useless inside a 5-minute window), FOK and FAK (default for market orders). postOnly is only valid with GTC/GTD. Prices must match tick size (0.01 here). Min size is 5 shares. neg_risk selects the exchange contract and is false for these markets.  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/trading/place-orders.md>, <https://clob.polymarket.com/neg-risk?token_id=... (live)>  

- CLOB protocol is V2 (GET https://clob.polymarket.com/version returns {version:2}). V2 signed orders contain salt, maker, signer, tokenId, makerAmount, takerAmount, side, signatureType, timestamp (ms), metadata and builder. Exchange domain version is '2'.  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://clob.polymarket.com/version (live)>, <https://docs.polymarket.com/trading/place-orders.md>  

- The official Rust SDK is crate polymarket_client_sdk_v2 (latest 0.8.0, 2026-09-10; ~288k downloads; repo Polymarket/rs-clob-client-v2, created 2026-04-08, last push 2026-10-19, 147 stars, 79 open issues, MSRV 1.88). It has clob/ws/gamma/data/rtds/heartbeats/ctf/bridge features but no PolyBolt and no session keys. The old Polymarket/rs-clob-client is archived. Crate 'polymarket-client-sdk' 0.4.4 (2026-03) is the older v1 crate. **[crucial]**  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://github.com/Polymarket/rs-clob-client-v2>, <https://crates.io/crates/polymarket_client_sdk_v2>, <https://github.com/Polymarket/rs-clob-client>  

- The Rust SDK README and examples use host https://clob-v2.polymarket.com, which does not resolve in DNS as of 2026-10-26. Use https://clob.polymarket.com. Open issues report a WebSocket resubscribe infinite-reconnect bug and custom_feature_enabled problems.  
  _Confidence: medium; source date: 2026-10-26_  
  Sources: <https://github.com/Polymarket/rs-clob-client-v2>, <https://github.com/Polymarket/rs-clob-client-v2/issues>, DNS lookup 2026-10-26  

- Heartbeat kill-switch: POST /heartbeats (L2 auth) every 5s. After 10s without one, a scan (up to about 5s later) cancels all open orders for that API key, best-effort. Once armed it cannot be disarmed, and WebSocket keepalive does not count.  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/api-reference/trade/send-heartbeat.md>  

- Trading rate limits are per signer token buckets. Standard tier: 40 orders/s (burst 60) and 80 cancels/s (burst 120); higher tiers based on 30-day volume. Over-limit requests get 429 with Retry-After (enforced after a 2-week warning period from 2026-07-24). POST /orders takes at most 15 orders and is all-or-nothing on rate limit. Public REST is IP-limited via Cloudflare (e.g. /book 1500/10s, Gamma /markets 300/10s), which delays requests rather than rejecting them.  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/api-reference/trading-rate-limits.md>, <https://docs.polymarket.com/api-reference/rate-limits.md>, <https://docs.polymarket.com/api-reference/trade/post-multiple-orders.md>  

- Matching engine restarts are announced about 2 days ahead on Telegram/Discord. During a restart orders get HTTP 425; afterwards there are 2 minutes where only post-only orders are accepted (503 post_only_mode). 'Trading is disabled' also returns 503.  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/trading/matching-engine.md>  

- Polymarket's primary servers are in AWS eu-west-2 (London). The closest non-georestricted region is eu-west-1 (Ireland). Co-location in eu-west-2 is available after KYC/KYB. **[crucial]**  
  _Confidence: high; source date: 2026_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>  

- Geoblock: the UK (GB), DE, FR, PL, IT, BE, SK, US, AU, SG and others are close-only for API orders. IE and NL are blocked on the frontend only, so API trading works. Serbia (RS) is not listed. A live GET https://polymarket.com/api/geoblock from the user's machine returned blocked:false, country RS. Orders from blocked IPs are rejected, so a VPS in London or Germany (e.g. Hetzner DE) would not work. **[crucial]**  
  _Confidence: high; source date: 2026-10-26_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>, <https://polymarket.com/api/geoblock (live)>  

- Measured from the user's machine in Serbia (via Cloudflare's Belgrade POP): CLOB REST /time took about 80-160 ms per request including TLS setup. This is small next to the 250 ms taker delay.  
  _Confidence: medium; source date: 2026-10-26_  
  Sources: live curl measurement 2026-10-26  

- Winning shares must be redeemed to turn back into pUSD; resolution docs say they become redeemable for $1. Redeem, split, merge and approvals can be done gas-free through https://relayer-v2.polymarket.com with a Relayer API key, or through the Rust SDK's ctf feature. Docs do not say redemption is automatic.  
  _Confidence: medium; source date: 2026_  
  Sources: <https://docs.polymarket.com/concepts/resolution.md>, <https://docs.polymarket.com/trading/gasless.md>  

### Risks

- The strategy compares live spot price against the window's start price, but settlement uses Chainlink 60s TWAP readings at window start and end. The bot must track the TWAP stream (crypto_prices_twap_sixty / PolyBolt price.crypto.twap), not Binance spot. A ~0.26% Binance–Chainlink gap was observed live.
- The 250 ms taker delay means a taker order sent at a >=92% price can be re-validated against a moved book and rejected, or filled after the information edge has passed. The order cannot be cancelled during the delay. Late-window latency arbitrage is structurally weakened.
- Legacy RTDS price topics are past their planned removal date (~2026-10-23) and could stop at any time. PolyBolt needs CLOB API credentials, so even paper trading needs a real account and API key. The Rust SDK has no PolyBolt client, so it must be hand-written.
- Giving the bot the owner private key (Magic export) gives it full withdrawal power. A compromised home server or VPS means total loss. Session Keys (trade-only) exist but need Builder API key approval and are not in the Rust SDK.
- Supply-chain risk: malicious npm packages posing as Polymarket tools stole private keys in 2026 (MAL-2026-4209). Pin and vet Rust crates; avoid unofficial polymarket-* crates.
- Cloud hosting in London (eu-west-2) or in close-only EU countries (DE, FR, PL, IT, BE, SK) gets orders rejected. Choose IE (eu-west-1), NL or another allowed country. Do not use a VPN to dodge geoblocks.
- The official Rust SDK is pre-1.0 (0.8.0) with open WebSocket reconnect bugs and a stale clob-v2 hostname in its README. Expect to patch it or bypass it for the WebSocket layer.
- Many small taker bets: the fee (~0.5% at p=0.92), the 5-share minimum and per-signer rate limits (40 orders/s on Standard tier) all cut into thin edges. Redeeming each winning position adds relayer operations.
- Matching engine restarts (425, then post-only mode) and 'trading disabled' 503s can hit mid-window. The bot must fail safe and reconcile positions over REST after WebSocket gaps.
- Heartbeat kill-switch, once armed, cancels all of that API key's orders if the bot stalls for ~10-15s. Mostly irrelevant for FAK/FOK taker orders, but it affects any resting GTC orders.

### Open questions

- Which wallet type does the user's actual account have? It depends on whether the account was created before or after 2026-05-04 and whether it used email or Google. Check the account wallet address and whether it is a Deposit Wallet (sig type 3) or a legacy Proxy (sig type 1).
- Does the key exported from reveal.magic.link/polymarket act as the owner/signer for a post-May-2026 Deposit Wallet? Docs don't say; verify by deriving API keys and signing a small test order.
- Exact semantics of the 'price to beat': is it the TWAP-60 value at window start, i.e. the average of the prior 60s? Which Chainlink report timestamp is used at window end? Confirm against resolved markets' final values.
- When will the legacy RTDS crypto_prices* topics actually be shut off, and does PolyBolt accept credentials derived from a deposit-wallet signer?
- Does the Rust SDK's ws module work reliably against the current CLOB (open resubscribe and custom_feature_enabled bugs)? Should the bot use a hand-rolled tokio-tungstenite client instead?
- Can an individual (non-builder) get Session Key access, and on what timeline? Session Keys would avoid giving the bot withdrawal power.
- Is the 250 ms taker delay (itode) applied to all 5m crypto assets (ETH, SOL, XRP) or only some? Query /clob-markets for each before trading.
- Are winning positions auto-redeemed for deposit wallets, or must the bot call redeem through the relayer after each window?

---

## 4. Legal, access and account risk for a Serbia-based user running an automated Polymarket (international) trading bot, as of Oct 2026

### Summary

ACCESS: Serbia (RS) is not on Polymarket's geoblock list. The official docs (undated, current at fetch on 2026-10-26) list four tiers. (1) Fully blocked: IR, SY, CU, KP, and the Crimea, Donetsk and Luhansk regions. (2) Close-only on both frontend and API: US, GB, DE, FR, IT, PL, SG, AU, BE, BR, RU, TW, TH, the Canadian provinces BC, ON, AB and QC, and others. (3) Close-only on the frontend only, with the API unrestricted: IE, JP, NL, KR, and MT (sports only). (4) Everyone else. A live call to polymarket.com/api/geoblock from a Serbian IP today returned blocked:false, country RS. Orders are checked by the requesting IP: 'Orders submitted from blocked regions will be rejected', and the error codes include 'address in closed only mode' and 'address banned'.

HOSTING: The docs say Polymarket's primary servers are in AWS eu-west-2 (London), and that the closest non-georestricted region is eu-west-1 (Dublin). London is in the UK, which is close-only on the API, so a VPS there cannot open positions. The one exception is co-location in eu-west-2, which Polymarket offers only to individual traders who complete its KYC/KYB form. Safe hosts are your home server in Serbia, AWS eu-west-1 (Ireland), Finland, Sweden, Serbia, and similar. Avoid Frankfurt, Germany generally (including Hetzner's DE sites), Paris, London, any US region and Singapore. Check the VPS IP with GET polymarket.com/api/geoblock on startup and on a schedule. If you live in Serbia (an allowed country), hosting in another allowed country is not geoblock circumvention, but this is my reading rather than an explicit Polymarket statement.

ToS/ACCOUNT: Using a VPN or proxy to evade geoblocks is prohibited (secondary sources cite ToS section 2.1.4). From May/June 2026, Polymarket reportedly blocks VPN IPs and pushes KYC, and accounts have been frozen or banned. API and bot trading is supported, with official SDKs and docs. The March 2026 Market Integrity Rules ban spoofing, wash trading and fictitious trades. The international entity is unregulated (footer: Adventure One QSS Inc.).

KEYS: Accounts created since 4 May 2026 use 'Deposit Wallets', which support Session Keys. These can trade only, cannot withdraw, last 180 days and can be revoked (beta; Builder API key needed). The bot should run on a session key, not the owner private key.

MONEY FLOW: The Polymarket card deposit runs through MoonPay, which requires KYC. MoonPay's payout table gives 'Rest of world' (where Serbia falls) Visa card payouts only. MoonPay lists Serbia for push-to-Visa-card payouts, and Exodus says Serbia is excluded from MoonPay bank-transfer sells. So a direct withdrawal to a Serbian bank account is likely unavailable. A Visa debit card payout is the likely fiat route. Serbia joined SEPA in May 2026, but I could not confirm whether MoonPay pays SEPA to Serbian IBANs. Fees are about 2-5% each way.

SERBIAN LAW: The Law on Games of Chance (Sl. glasnik 18/2020, 94/2024), Art. 10 t.3, prohibits taking part in foreign games of chance with stakes paid from Serbia. Art. 122 lets the regulator order IP blocking and blocking by banks and payment providers. I found no fine aimed at individual players, and it is untested whether prediction markets count as 'games of chance'. Neighbouring countries (HU, RO, PT, CZ, IT, FR, ES) have blocked Polymarket in 2026.

TAX: Profits on digital assets are taxed at 15% capital gains, with self-assessment within 120 days after the quarter ends, and crypto-to-crypto swaps reportedly count. Tax on prediction-market winnings in particular is unclear: they could count as digital assets at 15% or other income at 20%. Log every trade in RSD and see a Serbian tax adviser.

### Claims

- Serbia (RS) is not on Polymarket's blocked or close-only lists. The current official tiers are: fully blocked (IR, SY, CU, KP, plus Crimea, Donetsk and Luhansk); close-only on frontend+API (including US, GB, DE, FR, IT, PL, SG, AU, BE, BR, RU, TW, TH, NZ, SK, BY, the Canadian provinces BC, ON, AB, QC, and others); close-only on frontend only, with the API unrestricted (IE, JP, NL, KR, and MT sports only). **[crucial]**  
  _Confidence: high; source date: Undated official docs, fetched 2026-10-26_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>, <https://docs.polymarket.com/developers/CLOB/geoblock>, <https://docs.polymarket.com/polymarket-learn/FAQ/geoblocking>  

- A live call to GET https://polymarket.com/api/geoblock from a Serbian IP on 2026-10-26 returned blocked:false, country 'RS'. Serbia is currently allowed. **[crucial]**  
  _Confidence: high; source date: 2026-10-26 (live)_  
  Sources: <https://polymarket.com/api/geoblock>  

- Polymarket enforces geoblocks by the requesting IP, including on API order requests: 'Orders submitted from blocked regions will be rejected.' The CLOB returns a 400 error with 'address in closed only mode' or 'address banned'. Builders are told to call GET polymarket.com/api/geoblock, which returns blocked, ip, country and region. **[crucial]**  
  _Confidence: high; source date: Undated official docs, fetched 2026-10-26_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>, <https://docs.polymarket.com/resources/error-codes.md>  

- Polymarket's primary servers are in AWS eu-west-2 (London), and the closest non-georestricted region is eu-west-1 (Ireland). Only users who complete the KYC/KYB form can co-locate in eu-west-2. Co-location is limited to market makers and individuals trading their own accounts; it is not open to Builder Program participants or third-party apps. **[crucial]**  
  _Confidence: high; source date: Undated official docs, fetched 2026-10-26_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>  

- Safe cloud regions for opening positions are non-listed countries (Serbia, Finland, Sweden, Austria, Spain at the IP level, and so on) plus IE and NL, where the API is not restricted. AWS eu-west-1 Dublin is the documented closest option. A VPS in the UK (London), Germany (Frankfurt, Hetzner DE), France, Italy, Poland, the US, Singapore or Australia will be close-only on the API and cannot open positions. **[crucial]**  
  _Confidence: high; source date: Official docs undated (fetched 2026-10-26); Glassnode page undated_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>, <https://latency.glassnode.com/prediction-markets/about>  

- Using a VPN, proxy or anonymization tool to get around geographic restrictions breaks Polymarket's Terms (secondary sources cite section 2.1.4). The stated consequences include close-only mode, account bans and funds locked pending review. **[crucial]**  
  _Confidence: medium; source date: 2026-06-20 (tradetheoutcome); ToS text itself not retrievable (SPA page)_  
  Sources: <https://www.tradetheoutcome.com/polymarket-section-2-1-4/>, <https://www.benzinga.com/money/best-vpn-for-accessing-polymarket-in-the-u-s>  

- Since late May/June 2026, Polymarket has reportedly blocked IPs linked to VPN services and pushed international users toward KYC. Accounts flagged for VPN use are suspended or restricted. No deadline for mandatory international KYC has been announced. Polymarket's VP of engineering said the KYC requirement applied to a beta product. High-volume or rapid deposit-trade-withdraw cycles can trigger AML verification.  
  _Confidence: medium; source date: 2026-05-27 to 2026-06-01_  
  Sources: <https://www.covers.com/industry/polymarket-reportedly-tightens-vpn-rules-amid-rising-regulatory-pressure-june-1-2026>, <https://www.gncrypto.news/news/polymarket-tightens-kyc-after-isp-blocks-geoblocking/>, <https://www.cointelegraph.com/news/polymarket-kyc-sanctions-insider-trading-prediction-markets>, <https://coinspot.io/en/analysis/polymarket-blocks-access-via-vpn-and-introduces-kyc-should-you-be-worried/>, <https://finance.yahoo.com/markets/crypto/articles/polymarket-blocking-vpn-access-kyc-181022107.html>  

- No source says datacenter or cloud IP ranges are treated as VPNs. The docs explicitly point traders to eu-west-1, which suggests ordinary cloud hosting in an allowed country is accepted. **[crucial]**  
  _Confidence: medium; source date: Undated official docs_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock.md>  

- Automated and API trading is officially supported. Polymarket publishes CLOB API docs, SDKs, L1/L2 API-key authentication and rate limits. I found no ToS clause banning bots in the official docs. The ToS text could not be fetched (JS-rendered page).  
  _Confidence: medium; source date: Fetched 2026-10-26_  
  Sources: <https://docs.polymarket.com/trading/wallets-auth.md>, <https://docs.polymarket.com/llms.txt>  

- Polymarket's Market Integrity Rules, published around 20-23 March 2026, ban spoofing, wash trading, fictitious trades, front-running and insider trading on the international (DeFi) platform. Penalties include suspension, trade cancellation, wallet bans and referral to law enforcement.  
  _Confidence: high; source date: 2026-03-20/23_  
  Sources: <https://www.businesswire.com/news/home/20260320997513/en/>, <https://defirate.com/news/polymarket-publishes-enhanced-market-integrity-rules-for-us-defi-platforms/>, <https://thedefiant.io/news/defi/polymarket-tightens-insider-trading-rules>  

- Accounts deployed on or after 4 May 2026 use a 'Deposit Wallet'; older email/Magic accounts use a 'Proxy Wallet'. Deposit Wallet owners can authorize Session Keys. These can trade only ('cannot withdraw funds from the Deposit Wallet'), can be limited to CLOB, last 180 days, and can be revoked. They are in beta and need a Builder API key. The bot should hold a session key rather than the owner private key. **[crucial]**  
  _Confidence: high; source date: Fetched 2026-10-26_  
  Sources: <https://docs.polymarket.com/trading/session-keys.md>, <https://docs.polymarket.com/trading/wallets-auth.md>  

- Polymarket card deposits go through MoonPay, which converts to USDC and then to pUSD collateral. MoonPay is a money services business and requires KYC (government ID). The minimum purchase is about $20. Card plus FX costs are roughly 2-4%.  
  _Confidence: medium; source date: pm.wiki Apr-May 2026; MoonPay support undated_  
  Sources: <https://pm.wiki/es/learn/polymarket-deposit-guide>, <https://support.moonpay.com/en/articles/388702-moonpay-partner-faqs>, <https://support.moonpay.com/customers/docs/verify-my-account-details>  

- Withdrawing to a Serbian BANK account through MoonPay is probably unavailable. MoonPay's payout table gives 'Rest of world' Visa card payouts only. MoonPay's ETH sell page lists Serbia for push-to-Visa-card payouts. Exodus says Serbia is excluded from MoonPay bank-transfer sells. The realistic fiat route is a payout to a Serbian Visa debit card. The alternative is sending USDC to an exchange that serves Serbia and withdrawing from there. **[crucial]**  
  _Confidence: medium; source date: MoonPay support 2025-10-14; sell page undated; Exodus undated_  
  Sources: <https://support.moonpay.com/customers/docs/supported-bank-accounts-for-withdrawals>, <https://www.moonpay.com/sell/eth>, <https://www.exodus.com/support/en/articles/8598786-how-do-i-sell-crypto-with-moonpay-in-exodus>, <https://pm.wiki/es/learn/polymarket-withdrawal-guide>  

- Serbian banks began SEPA Credit Transfer operations in May 2026, with Banca Intesa starting 5 May 2026. This could open future EUR SEPA payouts to Serbian IBANs, but I found nothing confirming that MoonPay supports them.  
  _Confidence: medium; source date: 2026-03 to 2026-06_  
  Sources: <https://nbs.rs/en/scripts/showcontent/index.html?id=21592>, <https://www.bancaintesa.rs/en/stanovnistvo/obavestenje-o-primeni-SEPA-placanja.html>  

- Serbian Law on Games of Chance (Sl. glasnik RS 18/2020, 94/2024), Art. 10. Point 3 prohibits 'učestvovanje u igrama na sreću koje se priređuju u inostranstvu, za koje se ulozi plaćaju na teritoriji Republike Srbije', and points 19 and 20 prohibit enabling online games of chance by unlicensed operators. Under Art. 122(5) the inspector can order IP blocking by telecom operators and blocking of payment transactions by Serbian banks and payment providers. The penalties in Art. 132 (RSD 100k-2M) target legal entities and responsible persons; I found no fine for an individual player. Whether a prediction market counts as an 'igra na sreću' is untested. **[crucial]**  
  _Confidence: medium; source date: Law as amended 2024; RSE article 2020-11-16_  
  Sources: <https://www.paragraf.rs/propisi/zakon_o_igrama_na_srecu.html>, <https://www.slobodnaevropa.org/a/srbija-filtrira-internet-zbog-kladionica/30952526.html>  

- Since 15 October 2020, Serbia has had ISPs block unlicensed foreign online gambling sites at the regulator's request. I found no evidence that Polymarket is on a Serbian blocklist, and it is reachable from Serbia today.  
  _Confidence: medium; source date: 2020-11 (blocking regime); 2026-10-26 live check_  
  Sources: <https://www.slobodnaevropa.org/a/srbija-filtrira-internet-zbog-kladionica/30952526.html>, <https://polymarket.com/api/geoblock>, <https://startpolymarket.com/countries/serbia/>  

- In 2025-2026 several European regulators blocked or banned Polymarket as unlicensed gambling: Hungary and Portugal (Jan 2026), Czech Republic (Jul 2026), France (ISP block), Italy (ADM blocklist), Spain (proceedings and ISP block in May 2026), and the Netherlands (fine). ESMA warned in July 2026 about binary-options rules. Serbia is not in the EU but is a candidate and has the legal tools in Art. 122 to do the same.  
  _Confidence: high; source date: 2026-01 to 2026-07_  
  Sources: <https://decrypt.co/355144/polymarket-banned-portugal-hungary-prediction-market-regulatory-pushback>, <https://www.theblock.co/amp/post/408371/czech-republic-moves-to-block-polymarket-over-unlicensed-gambling>, <https://sigma.world/news/polymarket-in-europe-betting-bans/>, <https://publicgaming.com/news-categories-m/legal/16104-doors-close-for-prediction-markets-in-europe-polymarket-blocked-in-another-european-country>  

- For individuals in Serbia, gains from selling digital assets are capital gains taxed at 15%: sale price minus documented purchase price. A self-assessment return is due within 120 days after the end of the quarter in which the gain was made, and a return is reportedly needed even at a loss. Crypto-to-crypto swaps are reportedly taxable. Undocumented cost basis may be treated as zero. Reinvesting in a Serbian company within 90 days gives 50% relief.  
  _Confidence: medium; source date: Zunic Law updated 2026-05; N1 older_  
  Sources: <https://zuniclaw.com/en/cryptocurrency-and-digital-asset-tax-in-serbia/>, <https://n1info.rs/biznis/na-zaradu-od-trgovine-kriptovalutama-ne-placa-se-godisnji-porez-na-dohodak/>  

- How Serbia taxes Polymarket outcome-token profits is unresolved. They could be digital-asset capital gains (15%) or winnings or other income (20%, the rate on Serbian lottery-type winnings; online games from licensed domestic operators are exempt). I found no guidance specific to prediction markets. Consult a Serbian tax adviser.  
  _Confidence: low; source date: Various; no 2026 official guidance found_  
  Sources: <https://biznis.rs/vesti/srbija/kladionice-igraci-su-indirektno-oporezovani-preko-taksi-i-naknada-koje-placaju-priredjivaci/>, <https://zuniclaw.com/en/cryptocurrency-and-digital-asset-tax-in-serbia/>, <https://startpolymarket.com/countries/serbia/>  

- The international Polymarket platform is not regulated by the CFTC and runs separately from Polymarket US (QCX LLC, a CFTC-regulated DCM). The site footer names 'Adventure One QSS Inc.' Funds there have no regulatory deposit protection.  
  _Confidence: medium; source date: 2026 (footer)_  
  Sources: <https://polymarket.com/tos>, <https://finance.yahoo.com/markets/crypto/articles/polymarket-blocking-vpn-access-kyc-181022107.html>  

### Risks

- Cloud host in the wrong country: a VPS in London, Frankfurt or other German sites (including Hetzner DE), Paris, the US or Singapore gets close-only treatment on the API, so every new order fails. Run GET polymarket.com/api/geoblock at startup and periodically, and refuse to trade if blocked=true. Note that IP geolocation databases can misplace cloud IPs.
- Account freeze or ban if Polymarket's VPN detection mistakes a cloud or datacenter IP for a VPN, or sees a mismatch between the KYC country (Serbia) and the trading IP. Never use a VPN. Consider doing Polymarket's voluntary KYC so the account has a documented residence.
- Holding funds on an unregulated offshore platform: no deposit insurance, a fast-changing geoblock list, and possible AML holds on large or rapid deposit-trade-withdraw cycles. A high-volume bot fits that pattern.
- Exposing the owner private key: if the server is compromised, all funds can be stolen. Use Deposit Wallet Session Keys (trade-only, revocable) for the bot, and keep the owner key offline. Session keys are in beta and need a Builder API key.
- Fiat off-ramp: a direct withdrawal to a Serbian bank account is likely unsupported. A MoonPay Visa-card payout or a CEX route adds 2-5% in fees, separate MoonPay KYC, name-matching rules and possible 48h holds on a first large withdrawal. Serbian banks could also be ordered to block payments under Art. 122 of the Law on Games of Chance.
- Serbian legal risk: Art. 10 t.3 of the Law on Games of Chance bans taking part in foreign games of chance paid from Serbia. Whether Polymarket counts is untested, and no player fine was found. Regulators could add Polymarket to the ISP or payment blocklist, as HU, RO, CZ, PT, IT, FR and ES have.
- Tax and bookkeeping: with many small trades, every fill plus each deposit and withdrawal needs a timestamp, USD value and RSD value at the NBS rate, so quarterly self-assessment is possible. A missing cost basis may be treated as zero, making the whole sale price taxable.
- Integrity rules: wash trading, self-matching or fictitious trades can lead to bans. Make sure the bot can never trade against its own orders, for example when several strategies or accounts run at once.

### Open questions

- What does the current Polymarket Terms of Use (polymarket.com/tos, JS-rendered) say word for word about automated access, residence vs. IP location, and freezing funds? A human should read it in a browser.
- Does Polymarket's 2026 VPN detection flag ordinary cloud or datacenter IP ranges, such as AWS eu-west-1 or Hetzner Helsinki, for accounts registered in Serbia? Test with paper-mode order dry-runs and the geoblock endpoint before going live.
- Does the Polymarket withdraw screen show a Card/Bank (MoonPay) option for a Serbian account, and does MoonPay pay out EUR via SEPA to Serbian IBANs now that Serbia joined SEPA in May 2026? Check in-app.
- Do Serbian card issuers (Visa or Mastercard debit) accept MoonPay crypto purchases, and does Serbian foreign-exchange law (Zakon o deviznom poslovanju) or the Law on Digital Assets restrict residents buying crypto from unlicensed foreign providers? Ask a bank or lawyer.
- How does Serbia classify Polymarket profits for tax: capital gains on digital assets (15%) or other income or gambling winnings (20%)? Can losses on losing positions be offset? Consult a Serbian tax adviser (poreski savetnik).
- Does the Serbian Games of Chance Administration (uis.gov.rs) treat event-contract or prediction markets as games of chance, and is Polymarket on its unlicensed-operator list (uis.gov.rs/rsc/spisakpnaziv)?
- Is international KYC now mandatory for API traders, and do KYC-verified individuals get co-location in eu-west-2 London on request? This could cut latency for the last-30-second strategy.

---

## 5. Competition and strategy evidence for late-window high-probability ("endgame", sniping, latency arbitrage) strategies on Polymarket 5m/15m crypto Up/Down markets, as of 2026-10-26

### Summary

The core plan is to buy the 92%+ side in the last 30-45 seconds. The evidence says this is a crowded strategy in a bot-dominated market, and the raw edge is at best a few tenths of a percent per dollar, inside the noise. Three facts change the design.

1. Settlement is not "price at close" any more. The live market metadata I pulled today (btc-updown-5m, cryptoMarketConfig twapLookbackSeconds=60) and the official changelog both show 5m markets resolving on a 60-second Chainlink TWAP, in force since 2026-08-14. The bot has to model the TWAP. With 30-45 seconds left, half or more of the average is already fixed, and every competitor can see that too.
2. Crypto markets have a taker delay (`itode:true`): marketable orders are held and cannot be cancelled. The changelog says 150 ms since 2026-09-04; the order-lifecycle page still says 250 ms. Taker fees are 0.07·p·(1−p) per share, so break-even win rate is p + 0.07·p·(1−p), for example 92.5% at 0.92. Polymarket's servers are in AWS eu-west-2 (London). KYC'd individual traders can co-locate there, and the closest region without geo-restrictions is eu-west-1 (Dublin). From the user's network, the round trip to the CLOB measured about 80-90 ms.
3. Order books: I sampled 5 market closes. In 4 of them the favourite went to a bid of 0.99 with no asks at all 20-75 seconds before the end, with tens of thousands of shares queued to buy at 0.99. So 92-97¢ entries exist mainly in contested windows, which are exactly the ones that reverse. One BTC window went from 0.98 to 0.74 to 0.99 within 45 seconds.

My own 2-day on-chain check (576 windows per asset, a weekend, so low volatility) found trades at 0.92-0.97 in the last 45 seconds roughly calibrated: about +0.3% to +1.4% per dollar after fees for BTC, mixed in nearby price buckets, with 12 of 555 BTC markets reversing. The amount that can be traded is small, about $100k/day across all participants in BTC's 0.92-0.97 bucket. Independent backtests are negative: OpenMarket found no tradable edge, and a DEV backtest had an 82% win rate but a net loss. An open-source 5m bot whose author ran this exact late-favourite strategy live says it "does not make money."

A solo developer's realistic edge is better TWAP-aware probability modelling, careful choice of which markets to skip, and possibly maker (post-only) entries. Speed is not an edge here. The paper-trading phase must model queue position, fees, the taker delay and TWAP settlement, or it will report profits that don't exist. Also, the V1 Rust client is archived and no longer works; use rs-clob-client-v2.

### Claims

- 5-minute crypto Up/Down markets now resolve on a 60-second Chainlink TWAP compared against the price at the start of the window, not on a single close price and not on Binance. The live market description says the market is about the price on the Chainlink TWAP data stream and not about any other sources or spot markets. Gamma API metadata today shows cryptoMarketConfig {duration:'5m', twapEnabled:true, twapLookbackSeconds:60}. The changelog shows 30s TWAP from 2026-08-07, changed to 60s on 2026-08-14. **[crucial]**  
  _Confidence: high; source date: 2026-08-14 changelog; API checked live 2026-10-26_  
  Sources: <https://gamma-api.polymarket.com/events?slug=btc-updown-5m-1793026800>, <https://docs.polymarket.com/changelog.md>, <https://data.chain.link/streams/btc-usd-twap-60s-streams>  

- Many third-party articles and guides still describe snapshot settlement or a 30s TWAP for 5m markets. That information is out of date.  
  _Confidence: high; source date: 2026-08_  
  Sources: <https://cryptodaily.co.uk/2026/08/polymarket-replaces-snapshot-with-chainlink-twap>, <https://dev.to/casatrick/how-to-update-a-polymarket-trading-bot-for-twap-resolution-live-august-7-597c>, <https://docs.polymarket.com/changelog.md>  

- Polymarket moved to TWAP after a Stanford/SMU paper (Dai, Jia, Yu, 'Settlement Manipulation in Prediction Markets') found one-sided Binance order-flow bursts in the final seconds of 5m BTC windows. Net order flow in the last 10s was about 50% above pre-launch levels, and the price reverted afterwards. About 821 wallets captured an estimated $8.2M, and retail took 93% of the losses. Other coverage gives a $1.28M figure, so the dollar number is disputed. The effect was much weaker in 15m markets.  
  _Confidence: high; source date: 2026-07_  
  Sources: <https://arxiv.org/abs/2606.31675>, <https://unchainedcrypto.com/?p=45531>, <https://cointelegraph.com/news/stanford-study-5-minute-bitcoin-prediction-markets-enable-settlement-manipulation>, <https://business.smu.edu.sg/news/stanford-study-pins-polymarket-bitcoin-settlement-manipulation-contract-design>  

- Crypto markets currently have a taker delay. Marketable orders are held, cannot be cancelled during the hold, and are then re-validated. The changelog shows 250ms reduced to 50ms on 2026-08-17, then raised to 150ms on 2026-09-04. The order-lifecycle doc page still says 250ms. Live CLOB metadata for a current 5m BTC market shows itode:true. Earlier, a 500ms delay was quietly removed around 2026-02-18 (reported by Protos and others, with no changelog entry). **[crucial]**  
  _Confidence: high; source date: 2026-02 to 2026-09; checked live 2026-10-26_  
  Sources: <https://docs.polymarket.com/changelog.md>, <https://docs.polymarket.com/concepts/order-lifecycle>, <https://clob.polymarket.com/clob-markets/0x447eb28cc3b6222d3767825451c8af6efd806d0eeacc99de89cadfab2faad96a>, <https://protos.com/polymarket-ends-trading-loophole-for-bitcoin-quants/>  

- The current crypto taker fee is fee = shares × 0.07 × p × (1−p). It is taker-only, and 20% of it goes to maker rebates. Live market metadata shows feeSchedule {rate:0.07, exponent:1, takerOnly:true, rebateRate:0.2}. At p=0.92 the fee is about 0.515¢/share (0.56% of cost), so break-even win rate is about 92.5%. At 0.95 it is 0.33¢ (break-even about 95.33%). At 0.99 it is 0.07¢ (break-even about 99.07%). **[crucial]**  
  _Confidence: high; source date: 2026 (Fee Structure V2 from 2026-03-30); checked live 2026-10-26_  
  Sources: <https://docs.polymarket.com/trading/fees>, <https://gamma-api.polymarket.com/events?slug=btc-updown-5m-1793026800>  

- Taker fees were first added to 15m crypto markets on 2026-01-05/07, with a peak of 1.56% at 50¢. 5m markets launched on 2026-02-12 with fees already on. Fees were extended to all crypto markets on 2026-03-06, then to most categories with V2 on 2026-03-30. Officially, the fees fund maker rebates. Press and community commentary describe them as aimed at, or as reducing, latency arbitrage against stale quotes, but I found no official Polymarket statement saying that latency arbitrage was the target.  
  _Confidence: medium; source date: 2026-01 to 2026-03_  
  Sources: <https://docs.polymarket.com/changelog.md>, <https://www.theblock.co/post/384461/polymarket-adds-taker-fees-to-15-minute-crypto-markets-to-fund-liquidity-rebates>, <https://unchainedcrypto.com/polymarket-introduces-taker-fees-in-15-minute-markets/>, <https://cointelegraph.com/news/polymarket-quietly-adds-taker-fees-15-minute-crypto-markets>  

- It is widely reported that bots profited from Polymarket short-term crypto prices lagging Binance/Coinbase spot. The best-known case is a wallet reported to have turned about $313 into about $414k in a month on 15m BTC/ETH/SOL with a claimed ~98% win rate. These figures come from an analyst via BeInCrypto and Finance Magnates and are not independently verified. Reported lag measurements range from seconds (bot blogs) down to a 347ms median Polymarket quote response to large Binance moves (OpenMarket paper, Feb-May 2026 data).  
  _Confidence: medium; source date: 2026-01 to 2026-07_  
  Sources: <https://finance.yahoo.com/news/arbitrage-bots-dominate-polymarket-millions-100000888.html>, <https://www.financemagnates.com/trending/prediction-markets-are-turning-into-a-bot-playground/>, <https://arxiv.org/abs/2607.26245>  

- Fast markets are bot-dominated. In Dune's analysis (Sep 2025-Mar 2026), automated addresses made 55-62% of 5m/15m volume, while casual and regular traders made 4-5%. In 5m markets, 6,189 bot addresses made about 56M trades versus about 166k trades from 43,898 casual addresses. Bots averaged $6-7 per trade.  
  _Confidence: medium; source date: 2026-04_  
  Sources: <https://dune.com/blog/polymarket-fast-markets>, <https://blockchain.news/news/polymarket-fast-markets-volume-bots-fees-analysis>  

- Independent backtests of late-favourite strategies are negative. A one-day (2026-09-08) tick-level backtest of buying the 55-95¢ favourite in the last 60s of BTC 5m markets had an 82.4% win rate but a net loss of −$15.62 over 108 trades, because fees exceeded the 0.7¢ pre-fee edge. In the same test, a maker-quoting variant lost $26k to adverse selection, with negative markouts at every horizon. The OpenMarket paper (BTC 15m, 54 days) found that no strategy iteration survived fees and spread. **[crucial]**  
  _Confidence: medium; source date: 2026-07 to 2026-10_  
  Sources: <https://dev.to/levl/two-polymarket-strategies-that-look-fine-until-you-add-fees-and-a-queue-5g4p>, <https://arxiv.org/abs/2607.26245>, <https://github.com/gregyoung14/openmarket>  

- An open-source 5m bot that implements nearly the user's exact strategy (late entry on the favourite with ask between 0.85 and 0.95, Binance signal) states that it 'does not make money' after live small-stake trading. Reasons it gives: the price already encodes the probability; one reversal wipes out about 11 wins at 0.92; Binance and the settlement source disagree near the strike; taker fees plus the spread; and thin books near the close. **[crucial]**  
  _Confidence: medium; source date: repo pushed 2026-10-06_  
  Sources: <https://github.com/Shirotto/Polymarket-up-down-bot>  

- My own on-chain check (data-api taker trades, 576 consecutive 5m windows per asset ending about 2026-10-26 15:00 UTC, mostly a weekend) found BTC trades in the final 45s at 0.92-0.95 won 95.2% of shares (avg price 0.935, about +1.4% per dollar after taker fee). At 0.95-0.97: about +1.0%. At 0.90-0.92: −2.7%. At 0.97-0.98: −0.7%. At 0.99: +0.2%. 12 of 555 BTC markets had a late high-price trade that lost. ETH and SOL were similar but on thin volume. Total BTC volume at 0.92-0.97 in the last 45s was about 206k shares over 2 days, about $100k/day for all participants combined. The market is roughly calibrated and capacity is small. These are trades that actually happened, so the numbers carry selection bias. The sample is short and the windows are correlated. **[crucial]**  
  _Confidence: low; source date: 2026-10-24 to 2026-10-26 (own analysis)_  
  Sources: <https://data-api.polymarket.com/trades>, <https://gamma-api.polymarket.com/events>  

- Live order-book samples near the close (5 windows: BTC×2, ETH, SOL, XRP on 2026-10-26) showed the favourite reaching a 0.99 bid with no asks at all 20-75s before the end in most windows. The 0.99 bid queue grew from about 1k to 60k+ shares, and some bids sat at 0.991-0.999 because the tick size drops to 0.001 near extremes (minimum_tick_size 0.001). In one BTC window the favourite went 0.98 → 0.74 → 0.99 within the last 60s. Entries at 92-97¢ are mostly available in contested windows, which are the ones most likely to reverse. **[crucial]**  
  _Confidence: medium; source date: 2026-10-26 (own sampling)_  
  Sources: <https://clob.polymarket.com/book>, <https://clob.polymarket.com/markets/0x447eb28cc3b6222d3767825451c8af6efd806d0eeacc99de89cadfab2faad96a>  

- A wallet farming high-frequency 5m/15m markets ('gabigol') reportedly buys the leader at 94-99¢ in the last 1-2 minutes (about 48% of its buys), plus cheap underdog 'lottery' buys. Reported results: about $308k PnL on $20.6M volume, roughly a 1.5% margin, at about $2 median trade size. This is a single-wallet reverse-engineering post and is unverified.  
  _Confidence: low; source date: 2026-07 (year not shown)_  
  Sources: <https://dev.to/fatherson/reverse-engineering-gabigol-high-frequency-crypto-updown-farmer-on-polymarket-d0a>  

- Polymarket's primary servers are in AWS eu-west-2 (London). KYC'd market makers and individual traders trading their own account can co-locate there. The closest non-georestricted region is eu-west-1 (Dublin). The GB, DE, FR, IT and PL IP ranges are close-only (no new orders) for API trading too. Serbia is not listed as restricted. **[crucial]**  
  _Confidence: high; source date: 2026 (docs checked 2026-10-26)_  
  Sources: <https://docs.polymarket.com/api-reference/geoblock>, <https://docs.polymarket.com/developers/CLOB/geoblock>  

- The original Rust client Polymarket/rs-clob-client (and py-clob-client, clob-client) is archived. Its README says it is no longer functional and points to rs-clob-client-v2. CLOB V2 went live on 2026-04-28 with new order signing, pUSD collateral and no V1 compatibility. Open-source bots built on V1 clients will not trade without porting. **[crucial]**  
  _Confidence: high; source date: 2026-04 to 2026-10_  
  Sources: <https://github.com/Polymarket/rs-clob-client>, <https://github.com/Polymarket/rs-clob-client-v2>, <https://docs.polymarket.com/changelog.md>, <https://docs.polymarket.com/v2-migration>  

- Reference prices (Chainlink spot and 60s TWAP) are now served on the PolyBolt WebSocket (wss://ws-live-v2.polymarket.com/ws, channel price.crypto.twap with window_seconds 60, requires CLOB API credentials). The legacy RTDS price topics are deprecated, and the 30s TWAP topic has no replacement. **[crucial]**  
  _Confidence: medium; source date: 2026-09 to 2026-10_  
  Sources: <https://docs.polymarket.com/migrate/rtds-to-polybolt.md>, <https://docs.polymarket.com/changelog.md>  

- Prior art worth studying (last pushed dates from GitHub API on 2026-10-26): Polymarket/rs-clob-client-v2 (Rust, official, 2026-10-19); gregyoung14/openmarket (Rust Polymarket+Binance collector/backtester, archived research, 2026-07-31); mileswangs/pm-hftbacktest (Rust/Python tick backtester with queue and latency models, 2026-10-22); jtranq/polyfill-rs (Rust client, 2026-09-12); outcometick/polymarket-backtest (Python, fee- and queue-aware, 2026-10-20); Shirotto/Polymarket-up-down-bot (Python 5m bot with honest post-mortem, 2026-10-06); djienne/POLYMARKET_UP_DOWN_DERIBIT_STRATEGY_15m (option-implied TWAP probability model, 2026-09-21); EH-a0/polytread (small Rust 5m BTC service, 2026-07-22). Tick data is available from pmdata.dev (Polymarket books since 2026-05, Chainlink TWAP since 2026-06).  
  _Confidence: high; source date: 2026-07 to 2026-10_  
  Sources: <https://github.com/Polymarket/rs-clob-client-v2>, <https://github.com/gregyoung14/openmarket>, <https://github.com/mileswangs/pm-hftbacktest>, <https://github.com/jtranq/polyfill-rs>, <https://github.com/outcometick/polymarket-backtest>, <https://github.com/Shirotto/Polymarket-up-down-bot>, <https://github.com/djienne/POLYMARKET_UP_DOWN_DERIBIT_STRATEGY_15m>, <https://github.com/EH-a0/polytread>, <https://pmdata.dev/>  

- Binance spot and the Chainlink settlement price can differ near the strike. The Stanford paper reports Binance mid about 2.5 bps from the oracle. A developer claims gaps of 0.3-0.8% for 10-30s in volatile periods, which is unverified. Using Binance as the only signal is a known failure mode, and the bot should track the Chainlink/TWAP stream directly.  
  _Confidence: medium; source date: 2026-06 to 2026-07_  
  Sources: <https://unchainedcrypto.com/?p=45531>, <https://dev.to/lkto1m/why-my-polymarket-bot-watches-chainlink-not-binance-the-oracle-gap-that-changes-every-close-1e96>  

- The minimum order size on 5m markets is 5 shares. Tick size is 0.01 but drops to 0.001 near extremes. CLOB rate limits were raised in 2026, with POST /order at 120k per 10 min sustained.  
  _Confidence: high; source date: 2026-06; checked live 2026-10-26_  
  Sources: <https://clob.polymarket.com/markets/0x447eb28cc3b6222d3767825451c8af6efd806d0eeacc99de89cadfab2faad96a>, <https://docs.polymarket.com/changelog.md>  

### Risks

- The strategy has negative skew. At 0.92 a single reversal costs about as much as 11-12 wins. In my 2-day sample, about 2% of BTC windows had a late high-price trade that lost, and losses cluster in volatile periods (macro releases, liquidation cascades), which a weekend sample under-represents.
- Adverse selection. A taker order on a 92-97¢ ask is most likely to fill when faster participants have stopped defending that price, which is usually when the outcome is turning. Resting maker bids at 0.99 sit behind queues of 10k-60k+ shares and fill mostly when the price is falling.
- Liquidity. In most windows the favourite's asks are gone 20-75s before the close, so the intended 30-45s entry often has nothing to buy below 0.99. Total capacity in the 0.92-0.97 bucket is on the order of $100k/day for all BTC participants combined.
- Taker delay. Marketable orders are held for 150-250ms and cannot be cancelled, so the price can move against you before validation.
- Latency. From Serbia, round trips to the CLOB are about 80-90ms. Co-located competitors in eu-west-2 run in the low milliseconds, so the bot cannot win races.
- Settlement model mismatch. A bot that compares spot Binance price to the start price, instead of modelling the 60s Chainlink TWAP against the start reference, will misjudge probabilities, especially near the strike.
- Paper-trading illusion. If paper fills are taken at the displayed best ask or mid, with no queue, no taker delay, no walking the book, no fees and no Chainlink settlement, paper results will be overstated. The DEV backtest showed a maker strategy that looked profitable on price-touch fills losing $26k once queue position was modelled.
- Platform change risk. Fees, taker delay, TWAP window, client libraries (V1 to V2) and data feeds (RTDS to PolyBolt) all changed several times in 2026, often with little notice. Some changes, such as the February removal of the 500ms delay, were made silently.
- Cloud location. AWS London (eu-west-2) and VPS providers in DE, FR, IT, PL or GB geolocate to close-only countries, so orders from there are rejected. Dublin (eu-west-1) or a KYC co-location arrangement are the documented options.
- Most public 'profitable bot' content (Substack, Indie Hackers, Medium, bot vendors) is promotional and unverified. Headline win rates such as 98% do not imply profit at 0.92-0.99 entry prices.

### Open questions

- Does a TWAP-aware probability model (the observed portion of the 60s average, plus a volatility-based forecast of the rest) beat the market's price out of sample over weeks of data that include volatile weekdays? This needs a multi-week backtest on pmdata.dev or self-recorded books plus Chainlink TWAP data.
- What is the current taker delay on 5m crypto markets: 150ms (changelog, 2026-09-04) or 250ms (order-lifecycle doc)? Check per market with GET /clob-markets/{condition_id} and by measuring order round trips.
- Would post-only maker entries at 0.93-0.97, posted earlier in the window, have positive markouts after adverse selection, net of rebates (20% of crypto taker fees)?
- What fraction of windows actually offer a fillable ask at ≥0.92 and ≤0.97 at T-45s to T-30s, and what is the reversal rate in exactly those windows? My samples suggest they are the contested minority.
- Does KYC co-location in eu-west-2 work for a Serbian individual, and on what terms? Alternatively, what latency does Dublin (eu-west-1) give?
- Does Polymarket's Taker Rebate Program (tiered) materially change break-even for high-volume small takers? The tier details were not published on the fees page.
- How exactly does Chainlink compute the 60s TWAP (sampling rate, which exchanges are included, how the start reference is defined)? This is needed to replicate settlement precisely.

---

## 6. Completeness critique

**Completeness critique: what is missing or under-weighted, most crucial first (checked 2026-10-26)**

**1. There is no go/no-go test for whether the strategy has an edge, and it may well have none.**
- **Why it matters:** At 0.92 the bot needs about a 92.5% win rate after fees, and about 93.5% if it has to cross a 1-cent spread.
  - The evidence collected so far is mostly negative: independent backtests, the open-source bot's own "does not make money" note, and Bitquery's wallet data.
  - The only positive figure (+0.3% to +1.4% per dollar) comes from a 2-day weekend sample of correlated windows.
  - Measuring a win rate near 95% to within ±0.5 points takes about 1,900 independent trades (standard error ≈ sqrt(0.95×0.05/n)). Windows on the same day are correlated, so the real number is higher. At 100-300 trades a day, a few days of paper trading prove nothing.
- **Recommendation:** Write down the pass/fail rules before any code exists:
  - at least 4 weeks of recorded data that include volatile weekdays (FOMC, CPI, liquidation days);
  - the lower bound of the edge after fees and spread must be above 0;
  - calibration reported separately for each price bucket (0.92-0.95, 0.95-0.97, 0.97-0.99);
  - profit measured per window, not per trade.

  If those rules fail, stop or change strategy. Don't add more volume.

**2. Build a data recorder first. It is the foundation, not a nice-to-have.**
- **Why it matters:** Nobody has measured the order book depth at T-45 to T-30 seconds. The single snapshot could not be reproduced. Chainlink does not publish its TWAP method; Polymarket's docs say the sampling, weighting, rounding and handling of missing data are unpublished.
  - So the TWAP estimator and the fill model can only be calibrated from your own logs.
  - Ticks you did not record are gone. There is no affordable source to backfill them.
- **Recommendation:** Start a 24/7 recorder before any strategy work. It should store:
  - the CLOB market WebSocket (full book plus deltas) and trades for BTC, ETH and SOL 5m;
  - PolyBolt `price.crypto` and `price.crypto.twap`, keeping the original `payload.timestamp` and `seq`/`dropped`;
  - Binance BTCUSDT and BTCUSDC, Coinbase BTC-USD, and USDT/USD;
  - per-market config (`feeSchedule`, `itode`, `mts`, `mos`, `twapLookbackSeconds`);
  - the final resolution (`eventMetadata.finalPrice` / `priceToBeat`, winner, the time the CLOB showed `closed`).

  Store each event with both the exchange timestamp and the local receive time in nanoseconds, in append-only Parquet or ClickHouse. Then check, over hundreds of windows, whether the last PolyBolt TWAP equals `finalPrice` exactly. If it does not, the core model is built on the wrong number.

**3. Naive paper trading will show profits that are not there. The fill simulator has to be specified up front.**
- **Why it matters:** Several sampled windows had no asks below 0.99 some 20-75 seconds before the close. A 92-97 cent entry is mostly available in contested windows, which are the ones that reverse. One DEV backtest went from looking profitable to minus $26k once queue position was modelled.
- **Recommendation:** The simulator should apply all of the following:
  - decision time + measured round trip (about 80-90 ms from Serbia) + 150 ms taker delay, then fill against the book at that later moment, not the book you decided on;
  - walk the book level by level, using FAK/FOK semantics;
  - reject the order if the price has moved past the limit;
  - minimum 5 shares;
  - fee = shares×0.07×p×(1-p), rounded to 5 decimals;
  - read the tick from the CLOB, not Gamma, since it switches to 0.001 near extreme prices;
  - for maker orders, put the order at the back of the queue and count it filled only when traded volume at that price exceeds the queue ahead of it;
  - log every order you could not have filled. The "would have wanted but couldn't get" rate is a key metric.

  Then add a stage 1.5: live trading at minimum size for 1-2 weeks, comparing every real fill with what the simulator predicted for the same moment. One low-credibility report says accounts were credited 1-2% fewer shares than the order response stated. Check for that.

**4. Settlement and timing details the design still ignores.**
- **Why it matters:**
  - **What settles the market:** settlement compares the end TWAP with the previous window's final TWAP, using `>=` (a tie resolves Up). Binance spot is off by a USDT/USD basis of about 0.18%.
  - **How much is still open:** at T-45s only about 15 of the 60 seconds in the average are fixed. At T-30s about half are.
  - **No price to beat yet:** `priceToBeat` can still be null 235 seconds into a window if the previous window has not resolved.
  - **Clock:** window boundaries are Unix-time multiples of 300 seconds. One search summary says Polymarket's Chainlink-TWAP docs warn the server clock must be within 5 seconds of Chainlink's. I could not see that on the English page, so confidence is low. The design still needs disciplined time either way.
- **Recommendation:**
  - Run chrony with several sources and alert if the offset exceeds 50 ms.
  - Never compute window boundaries from receive times.
  - Take the price to beat from your own recorded PolyBolt TWAP at the boundary, and compare it with Gamma later.
  - Model P(final TWAP ≥ strike) as: the observed part of the average plus a volatility forecast for the remaining seconds, with Binance and Coinbase USD as the leading signal, converted to USD.
  - If the TWAP feed is stale (older than about 3 seconds, or gaps in `seq`), skip the window.

**5. Risk management is not designed at all.**
- **Why it matters:** The losses are lopsided. One loss at 0.92 wipes out about 11 wins, and one at 0.97 about 35. Losses cluster: one wick hits BTC, ETH and SOL in the same window at once. With the edge unknown, Kelly sizing does not apply.
- **Recommendation:**
  - Use a fixed small stake per trade (for example 0.5-1% of bankroll) until there are 2,000 or more real trades.
  - Treat all assets in the same 5-minute window as one bet and cap the combined exposure per window.
  - Set a daily loss cap (for example 3-5% of bankroll) and a weekly cap.
  - Trigger the kill switch automatically on any of:
    - N losses in a row;
    - realized win rate in a bucket drifting more than X standard deviations below the model;
    - stale feeds, or a clock offset above the threshold;
    - geoblock returning `blocked:true`;
    - a market config differing from expected (fee, `itode`, TWAP lookback);
    - balance mismatch on reconciliation.
  - Fail closed. The kill switch should cancel everything, stop new orders and send an alert.

**6. Key custody: the plan to hand the owner private key to the bot is the biggest security hole.**
- **Why it matters:** The owner key can withdraw everything. A home server or VPS compromise, a leaked log, or a malicious crate (fake Polymarket packages stole keys in 2026) means total loss. Session Keys (trade-only, revocable) exist but are in beta, need Builder approval, and are not in the Rust SDK.
- **Recommendation:**
  - **Phase 1 (paper) does not need the private key.** Derive the CLOB API credentials once on a separate machine and give the bot only those credentials, for PolyBolt and read access.
    - Placing an order needs a signature from the private key, so these credentials alone cannot trade. They can still cancel orders, so keep them secret too.
  - **Live phase:** apply for a Session Key now, given the approval delay. Until it arrives, keep only a small trading float in the account the bot controls.
  - Keep keys out of the repo and out of `.env` files in the project directory. Load them at runtime from an encrypted store (sops/age, or the OS keyring) into `secrecy`/`zeroize` types, and never log them.
  - Don't paste the key into any AI chat, including this one. Configure it on the server directly.
  - Run `cargo-deny`/`cargo-audit` and pin crate versions. Use only the official `polymarket_client_sdk_v2` crate.
  - The dashboard must not be reachable from the internet (use Tailscale or WireGuard) and must have no endpoint that places orders.

**7. Order state and reconciliation details.**
- **Why it matters:**
  - A trade moves through MATCHED → MINED → CONFIRMED, and can also go to RETRYING → FAILED. A position is not final until CONFIRMED.
  - A held taker order cannot be cancelled during the 150 ms delay, and the API waits to return the final result.
  - Order uniqueness depends on a millisecond timestamp, so blindly retrying after a timeout risks a double fill.
  - The market keeps accepting orders after the window ends.
  - Matching-engine restarts return 425, followed by about 2 minutes where only post-only orders are accepted.
- **Recommendation:**
  - Set HTTP timeouts above 150 ms + round trip + margin (for example 2 s).
  - After a timeout, query open orders and trades by market before any retry.
  - Hold one state machine per order that tracks trade status through to CONFIRMED or FAILED.
  - Reconcile positions and balance with the Data API after every window and on every WebSocket reconnect.
  - Hard-stop new orders at T-Xs by your own clock, and cancel any resting orders at the window end.
  - Arm the heartbeat endpoint (POST /heartbeats) if you ever rest orders.

**8. Capital locked between windows: redemption and accounting.**
- **Why it matters:** Winnings must be redeemed through the Relayer, which needs a Relayer API key. Resolution has a long tail (more than 2-4 minutes observed), so available balance is smaller than total equity.
- **Recommendation:**
  - Run an automated redeem loop with retries that waits for `STATE_CONFIRMED`.
  - Track three balances separately: free, in open positions, and resolved but not yet redeemed.
  - Size trades from free balance only.
  - Alert if anything stays unredeemed for more than 15 minutes.

**9. Card-in and bank-out could cost more than the trading edge, and bank-out may not exist.**
- **Why it matters:**
  - The MoonPay card onramp costs about 2-4%. The offramp costs about 3-5%.
  - MoonPay lists only Visa-card payouts for "rest of world", and I found nothing confirming payouts to Serbian banks.
  - Polymarket's withdraw doc only covers crypto withdrawal to an address. No source confirms card deposits work from Serbia either.
  - A 5-9% round trip on capital wipes out months of a 0.5%-per-trade edge.
- **Recommendation:** Before writing trading code, deposit about $20 by card and withdraw about $20 to your own bank or Visa card. Write down fees, holds and KYC steps. If there is no bank route, the "never touch crypto" requirement fails, because you would need an exchange that serves Serbia. Plan for rare, large withdrawals.

**10. Legal, tax and banking exposure in Serbia.**
- **Why it matters:**
  - Law on Games of Chance, Art. 10: point 3 bans taking part in foreign games of chance with stakes paid from Serbia; point 4 bans enabling payment of such stakes.
  - Art. 122 lets the regulator order IP and payment blocking. Neighbouring countries blocked Polymarket in 2026.
  - The NBS treats unlicensed digital-asset services as illegal for the provider. Whether that touches individuals using MoonPay is unclear.
  - A bank may flag recurring card payments to MoonPay.
  - Tax is either 15% capital gains or 20% other income, with quarterly self-assessment. It is unclear whether losses can be offset.
- **Recommendation:**
  - Book one paid consultation with a Serbian tax adviser and a lawyer before going live.
  - Build an automatic trade ledger from day one: every fill, fee, redemption, deposit and withdrawal, valued in USD and in RSD at the NBS middle rate.
  - Keep source-of-funds records for the bank.
  - Treat a sudden Serbian ISP or payment block as a scenario to plan for: get funds out quickly and don't keep a large balance on the platform.

**11. Hosting and geoblock details.**
- **Why it matters:**
  - Eligibility is based on physical location, not just IP.
  - The help center lists IE and NL as fully restricted, while the developer docs say their API works, so Dublin is contested.
  - Railway reportedly had all its regions blocked.
  - Polymarket's servers are in London (eu-west-2), which is close-only.
  - A home server means power and ISP outages and possibly a dynamic IP.
- **Recommendation:**
  - Stay in Serbia, or use a VPS in a country absent from both lists (Finland, Sweden or Austria).
  - Never use a VPN.
  - Call `GET polymarket.com/api/geoblock` at startup and every few minutes, and refuse to trade on `blocked:true`.
  - Add a UPS and a dead-man alert for the home server.
  - Measure the round trip from each candidate host, but don't pay for speed: speed is not where your edge is.

**12. Platform changes will silently break assumptions.**
- **Why it matters:** In 2026 alone, the taker delay went 250 → 50 → 150 ms, the TWAP window 30 → 60 s, fees changed 3 times, the CLOB moved from V1 to V2 (V1 clients stopped working), and price feeds moved from RTDS to PolyBolt. RTDS is past its planned removal date and still running. Some docs pages are stale; the order-lifecycle page still says 250 ms.
- **Recommendation:**
  - Fingerprint each market's config at runtime and halt on any unexpected difference.
  - Diff docs.polymarket.com/changelog daily.
  - Build only on PolyBolt and the CLOB WebSocket. Write your own tokio-tungstenite PolyBolt client, because the SDK has none.
  - Expect to patch the pre-1.0 Rust SDK's WebSocket resubscribe bugs.

**13. Data access hygiene.**
- **Why it matters:** REST polling of `/book` returned 403 in this research, possibly bot protection. Order and cancel rate limits are per signer.
- **Recommendation:** Use the WebSocket market channel for books, not polling. Don't spoof user agents. Honor 429 responses with exponential backoff and watch for the `Poly-RateLimit-Warning` header.

**14. Monitoring has to push alerts, not just display a dashboard.**
- **Recommendation:** Send Telegram or Pushover alerts for: kill switch, stale feed, geoblock change, reconciliation mismatch, positions not redeemed, clock offset, and daily profit and loss. Have an external watchdog alert you if the bot process is silent for more than 60 seconds.

**15. Paper and live must run the same code.**
- **Recommendation:** Keep one strategy core that reads a single event stream and writes order intents to an executor interface. Implement it three ways: replay (backtest from the recorder), paper (live data with the simulated fill model) and live. Write deterministic replay tests for the window maths, the TWAP estimator, fee rounding and the tie rule.

**16. Rethink the entry timing and the order type.**
- **Recommendation:** Use the recorded data to compare:
  - (a) taking at T-30 to T-45s, which is often impossible (no asks below 0.99) or adversely selected;
  - (b) entering at T-60 to T-120s, where a TWAP-aware model has more to add;
  - (c) post-only maker bids at 0.93-0.97, which pay no fee but face queue position and adverse selection.

  Include "skip contested windows" as a filter. Leave rebates out of the base economics; Polymarket can change them at any time.

**17. Integrity rules.**
- **Recommendation:** Never run two accounts or strategies that could trade with each other. Self-trade prevention is not documented, so guard against it in your own code. Spoofing, wash trading and self-dealing are banned under the March 2026 rules, and violations can lead to suspension or trade cancellation.

Sources:
- [Polymarket Chainlink TWAP docs](https://docs.polymarket.com/market-data/chainlink-twap)
- [Polymarket changelog](https://docs.polymarket.com/changelog.md)
- [Polymarket order lifecycle](https://docs.polymarket.com/concepts/order-lifecycle.md)
- [Polymarket rate limits](https://docs.polymarket.com/api-reference/rate-limits)
- [Polymarket integrity rules (BusinessWire, 2026-03-20)](https://www.businesswire.com/news/home/20260320997513/en/Polymarket-Publishes-Enhanced-Market-Integrity-Rules-Across-Its-DeFi-Platform-and-CFTC-Regulated-U.S.-Exchange)
- [Cointelegraph on Polymarket KYC](https://www.cointelegraph.com/news/polymarket-kyc-sanctions-insider-trading-prediction-markets)
- [KuCoin report of a CLOB outage (year unclear)](https://www.kucoin.com/news/flash/polymarket-experiences-major-outage-trading-temporarily-halted)
- [NBS warning on unlicensed crypto services (N1)](https://n1info.rs/biznis/nbs-upozorava-na-neovlasceno-poslovanje-bosanske-menjacnice-kriptovaluta-u-srbiji/)
- [pm.wiki deposit guide](https://pm.wiki/es/learn/polymarket-deposit-guide)
- [Casatrick on TWAP and latency](https://casatrick.substack.com/p/polymarket-twap-latency-trading-bots)

---

## Appendix: all fact-check verdicts

- **CONFIRMED**: Since 7 Aug 2026, crypto Up/Down markets (5m, 15m, 4h) resolve on Chainlink-computed TWAPs instead of a single price snapshot. Both the price to beat and the final price come from the same TWAP feed.
- **CONFIRMED**: 5-minute markets used a 30-second TWAP lookback from 7 Aug 2026 and moved to 60s on 14 Aug 2026 00:00 UTC. Live config shows btc-5m-twap-60 with twapLookbackSeconds 60; 15m shows btc-15m-twap-60.
- **CONFIRMED**: Rule text: Up if the Chainlink TWAP for the range is >= the price at the beginning of the range, else Down. A tie resolves Up. Resolution source is data.chain.link/streams/btc-usd-twap-60s-streams, and the rules say 'not according to any other sources or spot markets'.
- **CONFIRMED**: Price to beat of window N equals the final TWAP of window N-1 exactly. A third-party claim that the open is still an instantaneous snapshot is outdated or wrong.
- **CONFIRMED**: The final price is the TWAP of the last 60s of the window, so at T-30s about half of the average is already observed. The model must forecast the rest of the average. Third-party analysis says last-second sniping stopped working after the TWAP switch.  
  Correction: Nuance: the 'sniping ended' evidence comes from a working paper (not peer reviewed) cited by tradoxvps. It found the final-10-second share of Binance volume fell from 5.68% to 3.02%, but it could not separate the TWAP switch from liquidity rewards paid in the same month. Treat it as indicative only.
- **CONFIRMED**: Liquidity is concentrated in BTC. 24h volume on 2026-10-26: BTC 5m ~$12.3M, ETH ~$314k, SOL ~$221k, XRP ~$26k, DOGE ~$12k, BNB ~$5k, HYPE ~$1.4k, ZEC ~$30. Individual BTC windows trade about $33k-$100k.  
  Correction: Re-check at about 15:30 UTC: BTC $12.33M, ETH $322k, SOL $243k, XRP $26.6k, DOGE $12.1k, BNB $5.4k, HYPE $1.4k, ZEC $30. Per-window BTC volume in Gamma was $25k-$58k for most windows. Some windows showed near-zero volume, which looks like a lag in Gamma's per-market volume field. The figures are a rolling snapshot.
- **UNVERIFIABLE**: Order book snapshot at T-56s (single sample): BTC favorite top ~1,239 shares at 0.67, 1c spread, $150-$700 within 3c. ETH thinner with a 4c spread. SOL almost empty. Expect only hundreds to a few thousand dollars fillable per BTC window.  
  Correction: I could not reproduce this. My /book requests failed: one returned HTTP 403, and a later try returned a non-JSON body. One useful related finding: CLOB reports minimum tick size 0.001 near extreme prices, and prices of 0.9995 were seen after the window ended, while Gamma still listed orderPriceMinTickSize 0.01. Read the tick size from CLOB, not Gamma. Treat depth as unknown until the bot logs books itself during paper trading.
- **REFUTED**: Resolution latency: 15 sampled BTC windows had closedTime 78-80s after endDate. One was unresolved >133s after end. Bitquery reported median 26s and p90 85s (data to Aug 2026). Plan for 1.5-3+ minutes before capital is redeemable.  
  Correction: The Bitquery figures are correct: median 26s, p90 about 85s, data 1 Sep 2025 to 22 Aug 2026, verified 24 Aug 2026. Gamma closedTime is NOT a reliable measure of when resolution actually happened. It reads exactly end+80s for every window, including two that I watched stay unresolved for longer. Window 1793027400 ended 15:15:00, but at 15:19:13 Gamma still showed closedTime null and no finalPrice; later it was backfilled as 15:16:20. Window 1793027700 ended 15:20:00, and CLOB still showed closed=false and accepting_orders=true at 15:22:02; Gamma later recorded closedTime 15:21:20. Real resolution latency can exceed 2-4 minutes. Do not use closedTime for latency stats. Measure latency yourself from the CLOB closed/winner flags or on-chain resolution events. The '1.5-3+ min' budget is reasonable, with a longer tail.
- **CONFIRMED**: No trading cutoff before window end. The market keeps accepting orders after endDate until resolution (acceptingOrders=true, closed=false at end+51s). Trading stops at resolution. Bitquery documented large post-close 'gap' trading, mostly Apr-May 2026.  
  Correction: Strengthened: CLOB /markets showed accepting_orders=true, closed=false and an active book for window 1793027700 up to at least end+122s, with prices at 0.995/0.9995. Bitquery: about $23.1M won by gap bets, about 97% of it from mid-April to the second week of May 2026. Buying the near-certain side in the gap was negative every month since June 2026. Also, Gamma acceptingOrders/closed can disagree with CLOB, so use CLOB as the source of truth.
- **CONFIRMED**: Winning tokens are not auto-converted via the API. They must be redeemed with redeemPositions (gasless via Relayer with a Relayer or Builder API key) for 1 pUSD each, with no deadline. A bot needs an automated redeem loop. UI auto-redeem is unverified.  
  Correction: The docs say: 'There is no redemption deadline'. Each winning token returns 1 pUSD. Redemption has no amount parameter; it redeems both outcomes. It is submitted as a signed Deposit Wallet batch to the Relayer /submit endpoint, and you wait for STATE_CONFIRMED. Auto-redeem in the UI is still not documented officially, so it remains unverified.
- **CONFIRMED**: Collateral is pUSD (USDC-backed ERC-20 on Polygon) since the CLOB V2 cutover on 28 Apr 2026. Orders must use V2 signing (official Rust client rs-clob-client-v2 exists). Fees are set at match time, not in the order.
- **CONFIRMED**: Crypto taker fee = shares x 0.07 x p x (1-p), makers pay 0 and get a 20% rebate. Live feeSchedule {rate 0.07, exponent 1, takerOnly true, rebateRate 0.2}, feeType crypto_fees_v2. At p=0.92 the fee is ~0.00515/share (~0.56%), breakeven ~92.5%. At p=0.95 it is ~0.35%. The Feb 2026 1.56% peak is outdated (now 1.75%).  
  Correction: Clarification: the 1.75% peak is per $1 of payout ($0.0175 per share at p=0.5). As a share of cost at p=0.5 it is 3.5%. The old 1.56% was in the same per-share units.
- **CONFIRMED**: Taker orders on crypto Up/Down markets are held by a 150ms taker delay (since 4 Sep 2026; previously 250ms, then 50ms from 17 Aug) and cannot be cancelled during the hold. Checks re-run after. The order-lifecycle page still says 250ms and is stale.
- **CONFIRMED**: RTDS price topics are legacy since 15 Sep 2026, with removal planned about one month after SDK 0.11.0. The replacement is PolyBolt wss://ws-live-v2.polymarket.com/ws: price.crypto (Chainlink default since 2 Oct, optional Pyth pin) and price.crypto.twap (60s Chainlink TWAP, window_seconds must be 60). Both require CLOB API credentials. The 30s TWAP has no replacement.  
  Correction: Detail: Pyth pinning on price.crypto is available only for btc, eth, sol, xrp, doge and bnb. If a pin can't be served, the server silently falls back to the default, so check the 'subscribed' ack and the 'source' field. The server pings every 25s; two missed pongs close the connection with code 4002.
- **CONFIRMED**: Binance spot differs from the resolution source in venue (Chainlink aggregates multiple unnamed venues), quote currency (USDT vs USD) and method (60s TWAP vs instantaneous). Use Binance only as a leading indicator, calibrated against the PolyBolt Chainlink price/TWAP.
- **CONFIRMED**: Gamma eventMetadata.priceToBeat did not appear until well after the window started (filled when the previous window resolved). The bot should take the price to beat from its own TWAP stream at window start.  
  Correction: Stronger than claimed: for window 1793027700, priceToBeat was still null 235s into the window, because the previous window had not resolved yet. In normal cases it appeared about 80s or more into the window.
- **CONFIRMED**: Gamma marks these markets restricted=true. The geoblock endpoint returned blocked=false for a Serbian (RS) IP. Re-check from the actual server and any VPS region.
- **CONFIRMED**: Taker fee = C x 0.07 x p x (1-p), exponent 1, makers pay 0, fees in pUSD/USDC at match time rounded to 5 decimals (min 0.00001). Live btc/eth/sol 5m and btc 15m: crypto_fees_v2 {exponent 1, rate 0.07, takerOnly true, rebateRate 0.2}; clob-markets fd {r:0.07,e:1,to:true}.
- **CONFIRMED**: Fee/breakeven table: p=0.92 $0.00515, 92.52%, 12.4 wins per loss; p=0.95 $0.00333, 95.33%, 20.4; p=0.97 $0.00204, 97.20%, 34.8; p=0.99 $0.00069, 99.07%, 106.4. Fee is 0.07x(1-p) of notional but ~6.4-6.9% of each win's profit.  
  Correction: Recomputed independently, and all values match. The fee as a share of win profit is exactly 0.07 x p (6.44% at 0.92, 6.93% at 0.99). Small orders can lose a little extra to 5-decimal fee rounding.
- **REFUTED**: Taker delay is active (itode:true). Official docs say 250 ms; a third-party post claims it was raised from 50 to 150 ms on 2026-09-04, so the current value is uncertain. The original 500 ms delay was removed ~Feb 18 2026.  
  Correction: The value is not uncertain. Polymarket's official changelog says the crypto taker delay went from 250ms to 50ms on Aug 17 2026, then to 150ms on Sep 4 2026 at 14:00 UTC. The order-lifecycle page that says 250 ms is stale. Current value: 150 ms. The Feb 18 2026 removal of the 500 ms delay is backed only by secondary sources (Protos/cryptonews, 2026-02-20) and has no official changelog entry.
- **CONFIRMED**: 5m/15m markets resolve on a Chainlink TWAP: Up if TWAP >= start price, using the btc-usd-twap-60s stream. twapEnabled true, lookback 60. Started ~2026-08-07 (30s for 5m), moved to 60s ~2026-08-14. Much of the average is fixed in the last 30-45s, and Binance spot is not the settlement source.  
  Correction: Minor: at T-45s only about 15s of the 60s average is fixed, and at T-30s about half. 'Much' overstates it for T-45s.
- **REFUTED**: CLOB V2 went live 2026-04-28; V1 no longer works. The order struct changed (no nonce/feeRateBps; adds ms timestamp, metadata, builder) and the EIP-712 exchange domain version is '2'. Collateral moved to pUSD; API-only traders wrap via the Collateral Onramp. Official SDKs are TS and Python only, with no official Rust SDK, so Rust must implement signing itself or use a community crate.  
  Correction: The V2 facts are correct. The Rust part is wrong. Polymarket publishes an official Rust client, github.com/Polymarket/rs-clob-client-v2 (crate polymarket_client_sdk_v2, v0.8.0, authored by Polymarket Engineering, last pushed 2026-10-19). The Jul 17 2026 changelog also tells users to update their TypeScript, Python and Rust SDKs. The v2-migration page just doesn't mention it. Use the official crate; the V1 rs-clob-client is archived.
- **CONFIRMED**: Bank withdrawal from Serbia is uncertain. MoonPay SEPA payouts are for the EUR region only, with Visa card payouts for the rest of the world. Serbia is not on MoonPay's unsupported list; an Exodus article reportedly says MoonPay selling is unavailable in Serbia. Serbia joined SEPA scope 2025-05-22, with banks operational ~May 2026. Fallback: withdraw USDC to an exchange that serves Serbia.  
  Correction: The parts I could check hold. The MoonPay withdrawals table lists SEPA for EUR region and Visa only for rest of world, and Serbia is not named. Serbia is absent from MoonPay's unsupported list. The NBS and the EU Commission (5 May 2026) say 18 of 19 Serbian banks are live on SEPA Credit Transfer only, as indirect participants. The Exodus page returned 403, so that part is unverifiable. No official Polymarket help-center page on the bank off-ramp was found. Third-party guides put MoonPay cash-out at about 3-5% all-in. Treat bank cash-out as unconfirmed until tested in the Withdraw screen from Serbia.
- **CONFIRMED**: Serbia (RS) is not on the blocked or close-only list. Check via GET polymarket.com/api/geoblock. US, UK, DE, FR, IT, PL, SG, AU are close-only via the API; IE, NL, JP are frontend-only.  
  Correction: Also close-only on frontend and API: BE, SK, TW, TH, CA (BC/ON/AB/QC) and others. KR and MT (sports only) are frontend-only. Common EU VPS locations like Germany (Frankfurt), France and Poland are therefore unusable for placing new orders.
- **CONFIRMED**: Volume does not create edge; EV per trade = q - (p + fee). Bitquery found the crowd roughly break-even before fees and losing about the full fee after; fewer than a third of wallets were ahead in a sampled week; median about -$0.96 for one-market wallets vs -$101.61 for >500-market wallets.  
  Correction: Minor: Bitquery's table shows 37.2% of one-market wallets and 32.2% of 500+-market wallets ahead, while its text says 'fewer than a third' overall. Data runs 1 Sep 2025 to 18 Aug 2026, verified 19 Aug 2026, so it mostly predates the 60s TWAP regime.
- **CONFIRMED**: BTC 5m markets resolve on Chainlink's 60s TWAP stream: cryptoMarketConfig {btc-5m-twap-60, twapEnabled, 60}, resolutionSource btc-usd-twap-60s-streams. Up if end TWAP >= the start reading of the same stream.
- **OUTDATED**: These markets apply a 250 ms taker delay (itode:true). Marketable orders are held 250 ms, re-validated, then matched or booked. They can't be cancelled while held. A submission can return status 'delayed'.  
  Correction: The delay is 150 ms since 2026-09-04 14:00 UTC (it was 250 ms until Aug 17, then 50 ms). The order-lifecycle page's 250 ms is stale. itode:true was confirmed live, and the hold, re-validation and no-cancel behaviour are correct. On 'delayed': the docs describe that status for seconds-delay markets such as sports, so it is unconfirmed whether crypto taker-delay orders return it. Handle it defensively.
- **CONFIRMED**: Crypto fees: crypto_fees_v2, fee = shares x 0.07 x p x (1-p), taker-only, 20% maker rebate. GET /fee-rate returns base_fee 1000. V2 orders do not carry feeRateBps.
- **CONFIRMED**: Legacy RTDS (wss://ws-live-data.polymarket.com, PING every 5s) price topics crypto_prices, crypto_prices_chainlink and crypto_prices_twap_sixty are deprecated, with removal planned one month after SDK 0.11.0 (npm 2026-09-23). As of 2026-10-26 all three still streamed without auth at about 1 update/s per symbol.  
  Correction: The planned removal date (about 2026-10-23) has already passed, yet in a 20s test on 2026-10-26 about 15:30 UTC all three topics still streamed without auth. Removal could happen at any time. Don't build on RTDS; use PolyBolt.
- **CONFIRMED**: PolyBolt (wss://ws-live-v2.polymarket.com/ws) replaces RTDS price topics; channels price.crypto ({symbol:'btcusd'}, Chainlink default) and price.crypto.twap (window_seconds 60); auth frame with CLOB API creds required (unauth -> auth_required); server pings every 25s; 64 subscriptions; close codes 4001/4002/4003/4008; use full_accuracy_value decimal string.  
  Correction: Nuance: limits also include 20 subscribe frames/s, 64 KB frames and 8 auth frames, and any breach closes with 4008. RTDS is still needed for the 'activity' topic. On 2026-10-26 I reproduced the unauthenticated subscribe and got {op:error, code:auth_required}.
- **CONFIRMED**: Chainlink BTC/USD and Binance BTCUSDT differed by ~$200 (~0.26%); Binance is unsafe as a direct stand-in for settlement, use only as leading indicator.  
  Correction: The conclusion holds, but most of the gap is a systematic USDT/USD basis, not noise. My check at about 15:32 UTC on 2026-10-26: Chainlink BTC/USD 80,463; Binance BTCUSDT 80,605-80,612 (about 0.18%); Binance BTCUSDC 80,502; Coinbase BTC-USD 80,476; Kraken USDT/USD 0.9984. Converting BTCUSDT to USD, or using USD venues, brings the gap down to roughly $10-40. Settlement uses the 60s Chainlink TWAP, not spot, so you need to model the TWAP. The legacy RTDS Binance topic (crypto_prices) sent no updates in a 10s test, while crypto_prices_chainlink was still streaming.
- **CONFIRMED**: Signature types 0 EOA, 1 POLY_PROXY, 2 GNOSIS_SAFE, 3 DEPOSIT_WALLET (EIP-1271; 'Poly1271' in Rust SDK). All account wallets deployed on/after 2026-05-04 are Deposit Wallets. Funder = account wallet address from profile menu.  
  Correction: Detail: the docs say Proxy Wallet = 'Magic Link or Google authentication' and Safe = external signer such as MetaMask or Rabby. Deposit Wallets deployed after 2026-06-29 use a beacon proxy instead of UUPS. Connecting an existing account also involves a Relayer API key (Settings → API Keys → Relayer API Keys) for gasless operations.
- **UNVERIFIABLE**: A new card-funded email account created after May 2026 should trade with signature type 3 and funder = deposit wallet; owner key presumably the Magic-held key (inferred).  
  Correction: No primary source says who the owner/signer of an email-account Deposit Wallet is. Supporting evidence: py-clob-client-v2 issue #52 (2026-05) shows fresh EOAs rejected with 'maker address not allowed, please use the deposit wallet flow', so sig type 3 with funder = deposit wallet is very likely right. Read the signer address in the Relayer API key UI or client.account before trusting the Magic assumption.
- **CONFIRMED**: Email-account key export via https://reveal.magic.link/polymarket while logged in; article dated 2026-01-11; does not cover Google sign-in or deposit wallets.  
  Correction: The article was last updated 2026-01-11, before Deposit Wallets launched on 2026-05-04, so it may be stale for new accounts. It says it applies 'if you've signed up via email'.
- **CONFIRMED**: Session Keys (beta): separate EOA authorized for CLOB trading on a Deposit Wallet for fixed 180 days; cannot withdraw; creating requires owner key and Builder API key approved via builder@polymarket.com; not in Rust SDK.  
  Correction: The builder@ approval is described as needed 'during the initial rollout'. Session Keys work only with Deposit Wallets, and a migration flow for Proxy/Safe wallets is 'planned'. rs-clob-client-v2 main (v0.8.0) has no session-key code; only the TS and Python unified SDKs (@polymarket/client / polymarket-client) document authorizeSessionKey.
- **CONFIRMED**: Official Rust SDK polymarket_client_sdk_v2 latest 0.8.0 (2026-09-10), ~288k downloads, repo rs-clob-client-v2 created 2026-04-08, last push 2026-10-19, 147 stars, 79 open issues, MSRV 1.88; features clob/ws/gamma/data/rtds/heartbeats/ctf/bridge, no PolyBolt, no session keys; old rs-clob-client archived; polymarket-client-sdk 0.4.4 (2026-03) is v1 crate.  
  Correction: All numbers match: 288,024 downloads; features also include rfq and tracing. Caveats: the rtds module hardcodes the legacy wss://ws-live-data.polymarket.com, and PR #105 (Chainlink TWAP on RTDS) is still open. Polymarket's primary documented SDKs are now the unified TS/Python @polymarket/client (0.14.0, 2026-10-21); there is no unified Rust SDK.
- **CONFIRMED**: Primary servers AWS eu-west-2 (London); closest non-georestricted region eu-west-1 (Ireland); co-location in eu-west-2 after KYC/KYB.  
  Correction: The docs say only 'eu-west-2' and 'eu-west-1'; mapping these to AWS London and Dublin is an inference from AWS naming. py-sdk issue #195 (2026-07) reports the co-location KYB approval is opaque and slow: after 10+ days with no answer, the eu-west-2 IP stayed geoblocked as GB (close-only). Enforcement is per IP and needs an allowlist.
- **CONFIRMED**: Geoblock: GB, DE, FR, PL, IT, BE, SK, US, AU, SG etc. close-only for API orders; IE and NL frontend-only so API works; RS not listed; live geoblock returned blocked:false RS; VPS in London/Germany won't work.  
  Correction: This matches the developer docs, but the official help center article 'Geographic Restrictions' (updated 2026-08-14) contradicts them on IE and NL. It lists IE, NL, JP and MT among '39 countries completely restricted from accessing Polymarket'. Treat IE/NL as contested, not clearly allowed.
- **CONFIRMED**: Serbia not listed; tiers: fully blocked (IR, SY, CU, KP, Crimea, Donetsk, Luhansk); close-only frontend+API (US, GB, DE, FR, IT, PL, SG, AU, BE, BR, RU, TW, TH, NZ, SK, BY, CA-BC/ON/AB/QC, ...); frontend-only close-only (IE, JP, NL, KR, MT sports only).  
  Correction: This exactly matches the developer docs. The help center (2026-08-14) uses a different, older-looking taxonomy: it lists IE, JP, NL and MT as 'completely restricted', does not list KR, and names only SG, PL, TH and TW as close-only. Serbia appears in neither source. Help center: 'Permissions are based on physical location rather than residency.'
- **CONFIRMED**: Live GET polymarket.com/api/geoblock from Serbian IP on 2026-10-26 returned blocked:false, country RS; Serbia currently allowed.
- **CONFIRMED**: Geoblocks enforced by requesting IP including API orders ('Orders submitted from blocked regions will be rejected'); CLOB returns 400 'address in closed only mode' or 'address banned'; builders should call GET polymarket.com/api/geoblock returning blocked, ip, country, region.  
  Correction: Nuance: the error-codes page lists the two 400 strings as address-level states, and the docs do not map IP geoblock rejections to them. The exact error for an order rejected by IP geoblock is not documented. py-sdk #195 confirms enforcement is at the IP level.
- **CONFIRMED**: Primary servers eu-west-2 (London), closest non-georestricted eu-west-1 (Ireland); co-location only after KYC/KYB; limited to market makers and individuals trading own account, not Builder Program participants or third-party apps.  
  Correction: In practice approval can stall: py-sdk #195 had no response after 10+ days.
- **REFUTED**: Safe cloud regions: non-listed countries (Serbia, Finland, Sweden, Austria, Spain...) plus IE and NL (API not restricted); AWS eu-west-1 Dublin documented closest; UK/DE/FR/IT/PL/US/SG/AU VPS will be close-only.  
  Correction: The non-listed countries and the close-only list are right. Calling IE and NL 'safe' is not supported. The official help center (2026-08-14) lists IE and NL as completely restricted and says VPN-style bypassing breaks ToS 2.1.4, while the developer docs say their API is unrestricted. A Railway community post (2026-03-30) also reports Polymarket blocked all Railway regions, including EU West Amsterdam. Lower-risk choices are VPS regions in countries absent from both lists, such as Finland, Sweden or Austria, or staying in Serbia. Treat Dublin as a latency option whose eligibility should be checked with Polymarket support and re-checked live via /api/geoblock.
- **CONFIRMED**: Using VPN/proxy to bypass geo restrictions violates Polymarket Terms (section 2.1.4); consequences include close-only mode, bans and locked funds.  
  Correction: A primary source now exists. The official help center (updated 2026-08-14) says: 'Polymarket strictly prohibits the use of VPNs or similar tools to bypass geographic restrictions... violations of the platform's Terms of Service (Section 2.1.4).' The specific consequences (bans, locked funds) are still sourced only from secondary sources (Benzinga and similar).
- **CONFIRMED**: No source says datacenter/cloud IP ranges are treated as VPNs; docs pointing to eu-west-1 suggests ordinary cloud hosting in an allowed country is accepted.  
  Correction: I found no source saying datacenter IPs are blocked as such. Caveats: the help center says eligibility is based on physical location, not just IP. A VPS vendor (QuantVPS, updated 2026-10-12) notes that the geoblock result 'describes the requesting IP; it does not establish the operator's physical eligibility'. Railway users reported every Railway region blocked (2026-03), probably by country. The user being physically in Serbia, which is allowed, helps.
- **CONFIRMED**: Accounts deployed on/after 4 May 2026 use Deposit Wallet; older email/Magic use Proxy Wallet; Deposit Wallet owners can authorize Session Keys (trade-only, CLOB-scopable, 180 days, revocable, beta, need Builder API key); bot should hold a session key rather than owner key.  
  Correction: The facts are accurate. The recommendation is hard to apply now: Session Keys are beta, need builder@ approval of a Builder API key during rollout, are unavailable for legacy Proxy/Safe wallets, and are not implemented in the Rust SDK. You would sign the authorization via the TS/Python SDK or raw REST and EIP-712, and implement session-signer order signing yourself in Rust.
- **CONFIRMED**: Withdrawing to a Serbian bank via MoonPay probably unavailable; MoonPay 'Rest of world' = Visa card payouts only; Serbia listed for push-to-Visa; Exodus says Serbia excluded from bank-transfer sells; realistic fiat route = Visa debit card payout or USDC to an exchange serving Serbia.  
  Correction: The MoonPay support page (last updated 2025-10-14, older than 2026) gives Rest of world 'Visa only'. The MoonPay ETH sell page lists Serbia for Visa payouts. The Exodus page returned 403, so its Serbia exclusion is unverified. Polymarket's official 'How to Withdraw' (2026-01-12) documents only crypto withdrawal to an address (token, amount, network); no official doc confirms an in-app fiat or MoonPay off-ramp for Serbia.
- **CONFIRMED**: Serbian Law on Games of Chance (18/2020, 94/2024) Art. 10 pt 3 prohibits participating in foreign games with stakes paid in Serbia; pts 19-20 prohibit enabling online games by unlicensed operators; Art. 122(5) allows IP blocking and payment blocking; Art. 132 fines (RSD 100k-2M) target legal entities/responsible persons; no individual-player fine found; prediction market status untested.  
  Correction: The text matches the consolidated law. Art. 10 point 4 also bans enabling payment of stakes in Serbia for foreign games, which is relevant to card deposits. Art. 132(1)(5) penalizes legal entities for breaching Art. 10; responsible persons get RSD 5k-150k and entrepreneurs RSD 10k-500k for certain points. I found no 2025-2026 amendment.
- **CONFIRMED**: 5m crypto Up/Down markets resolve on a 60s Chainlink TWAP vs the start-of-window price, not single close and not Binance; Gamma shows cryptoMarketConfig {duration:'5m', twapEnabled:true, twapLookbackSeconds:60}; changelog 30s TWAP from 2026-08-07, 60s from 2026-08-14.  
  Correction: Nuance: the 2026-08-07 changelog says 'Both the price to beat and the final settlement price come from the applicable TWAP feed', so the start price is also TWAP-based, not a spot snapshot. The rule is '>=' (ties resolve Up). Live Gamma check (btc-updown-5m-1793028000) matched: resolutionSource btc-usd-twap-60s-streams, cryptoMarketConfig id btc-5m-twap-60.
- **CONFIRMED**: Crypto markets have a taker delay (held, uncancellable, re-validated); 250ms→50ms on 2026-08-17, →150ms on 2026-09-04; order-lifecycle page still says 250ms; itode:true live; earlier 500ms removed ~2026-02-18 without changelog (Protos).  
  Correction: The 150ms change took effect 2026-09-04 14:00 UTC and is logged under the Data API v2 changelog entry. The Protos article is dated 2026-02-20 and says the removal happened 'this month'. itode:true is confirmed on the current BTC 5m market 0xd4f3...40e8.
- **CONFIRMED**: Crypto taker fee = shares × 0.07 × p × (1−p), taker-only, 20% to maker rebates; at 0.92 ≈0.515¢/share (break-even ≈92.5%), 0.95 ≈0.33¢ (≈95.33%), 0.99 ≈0.07¢ (≈99.07%).  
  Correction: The math checks out. Missing items: (1) the Taker Rebate Program, live since 2026-05-28, pays tiered daily pUSD rebates on taker volume (crypto category weight 2.3; wV = size × (1−price) × weight), which lowers effective fees at high volume, though the (1−price) term makes 92¢+ trades earn little; (2) minimum order size is 5 shares (mos:5); (3) fees round to 5 decimals.
- **CONFIRMED**: Independent backtests negative: one-day (2026-09-08) BTC 5m last-60s 55-95¢ favourite backtest 82.4% win rate, −$15.62 over 108 trades (fees > 0.7¢ edge); maker variant −$26k with negative markouts; OpenMarket paper (BTC 15m, 54 days) found no strategy survived fees and spread.  
  Correction: The dev.to numbers match (fees −$52.90; maker variant −$26,133 over 1,105 trades), but the post shows 'Oct 13' with no year. The OpenMarket paper (data 2026-02-12 to 05-15) reports one walk-forward logistic model that did not beat book-implied probability (−0.116 payoff units per trade), so 'no strategy iteration survived' slightly overstates it.
- **CONFIRMED**: Open-source 5m bot (late favourite, ask 0.85-0.95, Binance signal) states it 'does not make money' after live small-stake trading; reasons: price encodes probability, one reversal wipes ~11 wins at 0.92, Binance vs settlement disagree, fees+spread, thin books.  
  Correction: The README matches. Credibility is low: the repo was created and pushed on 2026-10-06, has 0 stars, is Python, and has no backtester. It also reports being credited 1-2% fewer shares than the order response stated.
- **UNVERIFIABLE**: Own on-chain analysis of 576 windows: BTC last-45s 0.92-0.95 won 95.2% of shares (~+1.4%/$ after fee), other bands mixed, ~$100k/day capacity, roughly calibrated.  
  Correction: This is the requester's own analysis, which I did not replicate. A 2-day, weekend-heavy sample of correlated windows cannot separate +1.4% from zero. Treat it as a hypothesis for the paper-trading phase.
- **CONFIRMED**: Near-close books: favourite at 0.99 bid with no asks 20-75s before end in most windows; 0.99 queue 1k→60k+; tick drops to 0.001 near extremes; 92-97¢ entries mainly in contested windows.  
  Correction: My own sample on 2026-10-26 (BTC T−50 to T−33s, ETH T−25 to T−9s) showed Up at a 0.99 bid with 15k-41k shares and no asks. ETH tick switched to 0.001 by T−9s (0.999 bid). The older market 0x447e... shows mts 0.001. The contested-window point is an inference.
- **CONFIRMED**: Primary servers eu-west-2 (London), KYC'd MMs/individuals can co-locate; closest non-georestricted eu-west-1 (Dublin); GB, DE, FR, IT, PL close-only for API; Serbia not restricted.  
  Correction: The help center (2026-08-14) contradicts the developer docs by listing Ireland as completely restricted, so the Dublin recommendation is contested. Co-location approval is slow and opaque (py-sdk #195).
- **CONFIRMED**: rs-clob-client (and py-clob-client, clob-client) archived, README says no longer functional, points to v2; CLOB V2 live 2026-04-28 with new signing, pUSD collateral, no V1 compatibility; V1-based open-source bots won't trade without porting.  
  Correction: Additionally, resting orders from before the cutover were wiped. Fresh EOA (sig type 0) flows are reportedly rejected with 'maker address not allowed, please use the deposit wallet flow' (py-clob-client-v2 #52).
- **CONFIRMED**: Reference prices (Chainlink spot and 60s TWAP) now on PolyBolt (price.crypto.twap, window_seconds 60, needs CLOB API creds); legacy RTDS price topics deprecated; 30s TWAP has no replacement.  
  Correction: Timing risk: the docs say deprecated RTDS price topics are 'planned for removal one month after' @polymarket/client 0.11.0, released 2026-09-23, so removal is due around 2026-10-23. As of 2026-10-26 RTDS crypto_prices_chainlink still streamed, but the Binance crypto_prices topic sent nothing in my test. The Rust SDK's rtds feature uses the legacy endpoint, so build PolyBolt support yourself.
