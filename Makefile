.PHONY: db-up db-down backend-test backend-run frontend-dev dev test

db-up:
	docker compose up -d --wait db

db-down:
	docker compose down

backend-test:
	cd backend && cargo test --workspace

# caffeinate keeps the Mac awake while recording.
backend-run:
	caffeinate -is cargo run --release --manifest-path backend/Cargo.toml -p traderd -- --config config/traderd.toml

frontend-dev:
	cd frontend && pnpm dev

# Ctrl-C stops both; the database keeps running (make db-down).
dev: db-up
	$(MAKE) -j2 backend-run frontend-dev

test: backend-test
	cd frontend && pnpm test
