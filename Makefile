.PHONY: db-up db-down backend-test backend-run frontend-dev dev test poly-creds recompress

# rustup installs cargo here; the login shell may not have it on PATH.
export PATH := $(HOME)/.cargo/bin:$(PATH)

# CLOB API credentials for PolyBolt, written by `make poly-creds`. Lives outside the repo.
POLY_ENV ?= $(HOME)/.config/raki-trader/poly.env

db-up:
	docker compose up -d --wait db

db-down:
	docker compose down

backend-test:
	cd backend && cargo test --workspace

# caffeinate keeps the Mac awake while recording. PolyBolt turns on when $(POLY_ENV) exists.
backend-run:
	@if [ -f "$(POLY_ENV)" ]; then echo "loading PolyBolt credentials from $(POLY_ENV)"; set -a; . "$(POLY_ENV)"; set +a; \
	else echo "no $(POLY_ENV): running without PolyBolt (make poly-creds)"; fi; \
	caffeinate -is cargo run --release --manifest-path backend/Cargo.toml -p traderd -- --config config/traderd.toml

# One-off: derives the CLOB API credentials from the signer key (hidden prompt) into $(POLY_ENV).
poly-creds:
	cargo run --release --manifest-path backend/Cargo.toml -p raki-keytool --bin derive-creds -- --out "$(POLY_ENV)"

frontend-dev:
	cd frontend && pnpm dev

# Background, lowest CPU priority: recompresses finished hourly raw files (~30% smaller, lossless).
recompress:
	nice -n 19 cargo run --release --manifest-path backend/Cargo.toml -p traderd --bin recompress -- --watch data/raw

# Ctrl-C stops all three; the database keeps running (make db-down).
dev: db-up
	$(MAKE) -j3 backend-run frontend-dev recompress

test: backend-test
	cd frontend && pnpm test
