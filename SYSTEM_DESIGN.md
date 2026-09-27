# Hush — System Design Document

A URL shortener that takes a long URL and returns a short one. When someone visits the short URL, they get redirected to the original long URL. Think bit.ly, but ad-free, no login, no tracking — just the link.

This document explains **how** the system works, **why** we made each design choice, and **how** it scales.

---

## 1. Architecture Overview

```
                          +---------------------------+
                          |      Client (Browser)     |
                          |  - Shorten: POST /shorten |
                          |  - Redirect: GET /:code   |
                          |  - Stats: GET /stats/:code|
                          +------+------------+-------+
                                 |            |
                                 |            |
                                 v            v
                         +-------+------------+
                         |  Axum (Rust, Tokio) |
                         |  - Axum router       |
                         |  - Tower middleware   |
                         |  - Rate limiter (Redis)|
                         |  - Metrics (Prometheus)|
                         +---+--------+-----------+
                             |        |
                +------------+        +-----------+
                |                               |
          +-----+-----+                  +------+------+ 
          |  Redis    |  cache-aside     | PostgreSQL | 
          |  cache    |  (24h TTL)       |  (durability)|
          |  url:<code>→ long_url |      |  urls +     |
          |  ratelimit:<ip>       |      |  click_events|
          |  idempotency:<hash>   |      |  (outbox)    |
          +-----+-----+                  +------+------+
                |                               |
                +--------- OUTBOX ---------------+
                            |
                     +------+------+
                     | Analytics   |
                     | flush every |
                     | 5s (batch)  |
                     +-------------+
```

### Runtime Stack

| Layer | Technology | Why |
|-------|-----------|-----|
| Web | **Axum + Tokio** | Async Rust. No GC pauses. Single binary, tiny image. |
| DB | **PostgreSQL 16 + sqlx** | Durable, ACID, `SERIAL` + Base62 guarantees unique codes. `sqlx::migrate!` for versioned schema. |
| Cache | **Redis 7** | In-memory, cache-aside. Atomic Lua rate limiter path. |
| Observability | **Prometheus + Tracing** | `/metrics` + structured logs. `TRUSTED_PROXY_COUNT` gates XFF. |
| Deploy | **Docker (multi-stage, non-root)** | `Dockerfile` builds release binary; `public/` copied only in runtime stage. |

---

## 2. Request Flows

### 2.1 Shorten — `POST /shorten`

```
Client: POST /shorten { longUrl: "https://example.com/a/very/long/path" }
             |
             v
      1. Validate URL — must be http(s)://, cap at 2048 bytes.
             |
             v
      2. Rate limit — Redis Lua (atomic INCR+EXPIRE on ratelimit:<ip>,
         10/min). If over limit -> 429.
             |
             v
      3. Idempotency check (Dedup) —
         a) Idempotency-Key header? If seen in Redis, replay cached response.
         b) Otherwise: does long_url already exist in urls? If yes,
            return existing short_code (unique index on long_url).
             |
             v
      4. INSERT url -> get id (SERIAL). Encode id -> Base62.
         UPDATE short_code on that row. Wrap in a single DB transaction
         with ON CONFLICT (long_url) handling for concurrent racers.
             |
             v
      5. Cache the mapping in Redis: SETEX short_code -> long_url (86400s).
         (If Redis is down, step silently — DB is the truth.)
             |
             v
      6. If an Idempotency-Key was sent, cache the whole response in Redis
         with TTL = IDEMPOTENCY_KEY_TTL_SECONDS (3600).
             |
             v
         200 { shortUrl: "http://host/:code", shortCode: "b" }
```

**Why INSERT then UPDATE?** `SERIAL` gives a guaranteed-unique integer. Base62 of that integer is collision-free by construction — no "check if taken, try again" loop.

### 2.2 Redirect — `GET /:code`

```
Client: GET /abc
            |
            v
     1. Validate short_code format (alphanumeric only) -> 404 if bad.
            |
            v
     2. Check Redis: GET abc
            |
     +------+------+
     |             |
    HIT           MISS
     |             |
     |             v
     |      2b. SELECT long_url FROM urls WHERE short_code = abc
     |            - if not found -> 404
     |            - if found: SETEX abc -> long_url in Redis (repopulate)
     |             |
     +------+------+ 
            |
            v
     3. Record click — async: enqueue short_code into analytics queue.
        Do not wait for it. User already gets the redirect.
            |
            v
     4. 302 Found, Location: <long_url>
        (302, not 301 — so the browser doesn't cache it and we can count every click.)
```

**Why 302?** A 301 poisons the browser cache — the client never asks us again, so analytics go stale. 302 keeps every visit visible to the server.

### 2.3 Stats — `GET /stats/:code`

Read-through: try Redis (hit -> return), miss -> PostgreSQL, return `{ shortCode, longUrl, clickCount, createdAt }`.

### 2.4 Health — `GET /health`

Probes both backends in parallel (with 2s timeout per probe). Returns `{ status: "ok"|"degraded", redis, database }`. Used by Docker `HEALTHCHECK` and the navbar status pill. The frontend never shows "Unreachable" — that is a backend concern.

---

## 3. Data Model

### Tables

```sql
-- Primary mapping. One row per distinct long_url.
CREATE TABLE urls (
  id         SERIAL PRIMARY KEY,          -- auto-increment, fed to Base62
  short_code VARCHAR(10) UNIQUE NOT NULL, -- Base62(id), e.g. "1", "10"
  long_url   TEXT UNIQUE NOT NULL,        -- 2048 cap enforced in app
  click_count INT DEFAULT 0,              -- batch-updated by analytics flush
  created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE UNIQUE INDEX idx_urls_short_code ON urls(short_code);
CREATE UNIQUE INDEX idx_urls_long_url   ON urls(long_url);

-- Durable per-click events. The redirect path writes here, not into urls directly.
-- A single DELETE...RETURNING + batch UPDATE consumes them.
CREATE TABLE click_events (
  id         BIGSERIAL PRIMARY KEY,
  short_code VARCHAR(10) NOT NULL,
  created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX idx_click_events_short_code ON click_events(short_code);
```

Migrations are versioned and applied at boot via `sqlx::migrate!("./migrations").run(pool)`. `migrations/0001_initial.sql` and `migrations/0002_click_events.sql`.

### Base62

Alphabet `a-zA-Z0-9` (62 chars). No `+` nor `/`, so codes are URL-safe without escaping. `id=1 -> "1"`, `id=62 -> "10"`, `id=3844 -> "100"`.

---

## 4. Caching — Cache-Aside

1. Check Redis.
2. If miss, read PostgreSQL and repopulate Redis with `SETEX(code, longUrl, 86400)`.
3. If Redis is down, skip caching entirely and serve from PostgreSQL (graceful degradation).

- **TTL:** 24 hours (`CACHE_TTL_SECONDS=86400`).
- **Invalidation:** Expiry only (v1). No explicit `DELETE /cache` endpoint — the app is intentionally not exposing cache-flush to users (it would let anyone disrupt the cache).
- **What we cache:** short_code -> long_url only. No user data.

---

## 5. Rate Limiting — Redis + Lua

Fixed-window, per-IP, 10 req/min (`RATE_LIMIT_MAX_REQUESTS=10`, `RATE_LIMIT_WINDOW_MS=60000`).

The check is an **atomic Lua script**: `EVAL "INCR + set EXPIRE if first + compare to max"` so two concurrent requests from the same IP cannot both read the count before either increments it. If over limit -> `429 Too Many Requests`.

Why Redis, not an in-memory `Map`? A `Map` is per-instance — doubling behind a load balancer would silently double the limit. Redis is the one shared truth across instances.

XFF pinning: `client_ip` only trusts `X-Forwarded-For` when `TRUSTED_PROXY_COUNT > 0`. Otherwise it uses the socket IP, so an attacker cannot bypass the limit with a forged header.

---

## 6. Idempotency

Two layers, both Redis-backed:

1. **`long_url` dedup** — a `SELECT ... WHERE long_url = $1` before insert; a unique index on `long_url` makes it O(1). If two clients shorten the same URL concurrently, the DB `ON CONFLICT` returns the first row's code deterministically.
2. **`Idempotency-Key` header** — client-sourced, 1..128 chars, required to be present to get hit. The response for a key is cached in Redis for `IDEMPOTENCY_KEY_TTL_SECONDS` (3600).

This means: if a client times out and retries, we never create a duplicate short code. "Duplicate short codes for the same URL" is the anti-pattern Idempotency prevents.

---

## 7. Analytics — Durable Outbox

Each `GET /:code` enqueues a `click_events` INSERT **in the same transaction window** visible to the later flush. A background task runs every 5 seconds:

1. `DELETE FROM click_events RETURNING short_code` — atomically drains the outbox.
2. Group by short_code in memory.
3. `UPDATE urls SET click_count = click_count + count WHERE short_code = ...` — batch.

Properties:
- Crash-safe: `click_events` survives a restart (it's in PostgreSQL, not an in-memory Vec).
- Bounded: `DATABASE_MAX_CONNECTIONS` and `DATABASE_ACQUIRE_TIMEOUT_SECONDS` keep the pool from hanging.
- Non-blocking: the redirect never waits for the write — only the 5s-periodic flush touches `urls.click_count`.

**Trade-off v1:** Per-redirect `INSERT` is heavier than an in-memory increment. At very high throughput you'd move ingestion to Redis Streams / Kafka / SQS and keep a consumer that batches to Postgres (see §13).

---

## 8. Hardening (Production-Ready Checklist)

| Fix | File | What changed |
|-----|------|--------------|
| Analytics outbox transaction | `src/analytics.rs` | Single `DELETE...RETURNING` + `UPDATE` in one tx; non-root container owns `public/` |
| `ON CONFLICT` on insert | `src/shortener.rs` | Handles concurrent POST of same `long_url` without duplicate rows |
| `HeaderValue::from_str` fallible | `src/main.rs` | `match` on redirect header parse, not `.unwrap()` |
| XFF spoof gate | `src/config.rs`, `src/main.rs:client_ip` | Only trust XFF when `TRUSTED_PROXY_COUNT>0` |
| Body cap | `src/main.rs` | `RequestBodyLimitLayer(8KB)` on `POST /shorten` |
| Short code validation | `src/shortener.rs:is_valid_short_code` | Reject non-alphanumeric `/:code` with 400 |
| `long_url` length cap | `src/shortener.rs` | 2048 char cap at validation |
| Health timeout | `src/main.rs:health_handler` | 2s per-probe timeout, returns `degraded` |
| DB/Redis retry with backoff | `src/db.rs`, `src/cache.rs` | 10 retries, 3s sleep |
| `BASE_URL` sanitize | `src/config.rs` | Trim + strip trailing slash + validate |
| Pool tuning by env | `src/config.rs`, `.env.example` | `DATABASE_MAX_CONNECTIONS`, `DATABASE_ACQUIRE_TIMEOUT_SECONDS` |
| Non-root runtime | `Dockerfile` | `groupadd/useradd shorty`, `chown -R shorty:shorty /app` |
| Idempotency-Key validation | `src/idempotency.rs` | 1..128 chars, explicit 400 on invalid |
| Metrics bounded fallback | `src/metrics.rs` | Unmatched routes -> `unknown` / `/assets/*`, guarded PrometheusHandle |

---

## 9. Observability

- **`/health`** — probes Redis `PING` and `SELECT 1`. Consumed by Docker `HEALTHCHECK`, load balancers, and the navbar status pill.
- **`/metrics`** — `metrics_exporter_prometheus`. Posts a handler at `GlobalMark("prometheus")`. Labels `{ method, path, status, outcome }`, with bounded fallback so the cardinality cannot explode on 404 storms.
- **Tracing** — `tracing_subscriber::fmt` (JSON-free in Docker; plain text to stdout).
- **Graceful shutdown** — `tokio::signal` (Ctrl+C on unix, trap on Windows) -> `axum::serve(...).with_graceful_shutdown(...)`.

---

## 10. Frontend Architecture

Single-page app — one HTML file, one CSS bundle, one JS bundle.

```
public/
  index.html   # Structure: sticky nav + header (title/lede) + two-column split (Shorten + Inspect)
  shorty.css   # Single stylesheet: Vercel brand primitives (`.vbg-*`) + page-owned `vbg-custom-*` overrides
  app.js       # Fetch-based, no framework. Paste helper, form validation, toasts.

Assets
  public/assets/ascii-magic-*.png/.jpg  # Unused in the current Vercel-clean layout; safe to delete.
```

`index.html` has no inline scripts and no build step — the Rust `ServeDir/ServeFile` serves it statically. The brand CSS is not loaded as a separate `assets/vercel-brand.css` anymore; its bytes were merged into `shorty.css` so only one style request is made.

---

## 11. API Reference

### `POST /shorten`

```bash
curl -X POST http://localhost:3000/shorten \
  -H "Content-Type: application/json" \
  -d '{"longUrl":"https://example.com/a/very/long/path"}'
# Optional idempotency replay:
#  -H "Idempotency-Key: 6e8b0b2a-..." \
```
```json
{ "shortUrl": "http://localhost:3000/b", "shortCode": "b" }
```
- 400 on missing/invalid URL, bad Idempotency-Key, or >2048 payload.
- 429 if rate limited.
- Idempotent on `long_url` and on `Idempotency-Key`.

### `GET /:code` -> 302

```bash
curl -i http://localhost:3000/b
# 302 Found
# Location: https://example.com/a/very/long/path
```

### `GET /stats/:code`

```json
{ "shortCode": "b", "longUrl": "https://example.com/...", "clickCount": 3, "createdAt": "2026-09-02T..." }
```

### `GET /health`

```json
{ "status": "ok", "redis": "connected", "database": "connected", "timestamp": "..." }
```

### `GET /metrics`

Prometheus exposition format.

There is no `GET /api/urls` (recent link enumeration) or `DELETE /api/cache/*` in the current build — they would leak URLs or let anyone clear the cache, so they were intentionally removed.

---

## 12. Limitations

| Limitation | Why it exists | Severity |
|------------|---------------|----------|
| No custom aliases | Out of scope — auth + ownership needed. | Product — not security. |
| No expiring URLs | `expires_at` is gone from schema by intent. | Product. |
| No user accounts | No auth was added in this task. Scope was exactly six hardening items. | Product. |
| Single PostgreSQL | Not sharded; one instance is the bottleneck at very high write volume. | Scaling — see §13. |
| Single Redis | Not clustered; if Redis is down, every hit is a DB query. | Availability. |
| Per-redirect INSERT | Analytics does a `click_events` INSERT per visit before batching. | Performance at very high traffic. |
| Fixed-window rate limit | Not sliding/token-bucket; a burst at the boundary can pass slightly more. | Minor. |
| No global CDN yet | Redirect latency follows origin geography. | Latency at scale. |
| Long URL cap 2048 | Prevents abuse of an unbounded `TEXT` column. | Hard limit. |

---

## 13. Scaling This Further

> Written in first person, as a design-thinking signal — not built yet.

If Hush ever outgrows a single machine, I would **shard PostgreSQL by hashing the short_code** across several instances. A short_code never changes and is unique, so a hash of it routes every lookup to exactly one shard with no cross-shard join, and the redirect stays a single query.

Redirects vastly outnumber new-URL creation, so I would **put read replicas behind redirect traffic** and keep writes on the primary. The DB pool's `DATABASE_MAX_CONNECTIONS` and `DATABASE_ACQUIRE_TIMEOUT_SECONDS` already expose tuning; a replica pool would sit beside it. I would also **push redirects through a CDN / edge cache** — the short-code -> long-url mapping is stable after creation and nearly ideal for edge caching.

For analytics, the current outbox is durable (it survives a crash) but a raw per-click `INSERT` has a ceiling. Once redirect volume exceeds what PostgreSQL can absorb comfortably, I would **move ingestion to Redis Streams or a durable message queue (Kafka/SQS)** and keep a consumer that batches updates into the database. That keeps durability while removing write pressure from the redirect path.

---

## 14. Tests

```
cargo test                          # 6 Rust unit tests: base62 + URL validation
node scripts/verify.js              # 9 integration checks: health, HTML, assets, shorten,
                                     # idempotency, 302, stats, validation
```

Adding the load test's numbers is still pending: run `npx autocannon -c 100 -d 15 http://localhost:3000/<code>` once for a cache-hit code and once after `DEL <code>` in Redis, and fill the Performance table in `README.md`.

---

## 15. Project Layout

```
Cargo.toml           Rust manifest (Axum, sqlx, redis, tokio, ...)
Cargo.lock           Locked deps

migrations/          sqlx versioned schema (0001_initial.sql, 0002_click_events.sql)
src/
  main.rs            Router, AppState, health/shorten/redirect/stats handlers.
  config.rs          Typed config (env + truncation/XFF gates + pool tuning)
  db.rs              PgPool create + retry + initialize_database
  cache.rs           Redis MultiplexedConnection + retry + SETEX helpers
  shortener.rs       Base62, validators (short_code, long_url cap), save/get
  rate_limiter.rs    Redis+Lua fixed-window, per-IP, atomic
  idempotency.rs     Redis-backed key replay
  analytics.rs       click_events outbox + 5s flush
  metrics.rs         Prometheus recorder + route-normalized middleware

public/
  index.html         Single page (sticky nav + header + Shorten/Inspect split)
  shorty.css         Merged stylesheet (Vercel brand + page-owned)
  app.js             Fetch frontend (paste, validate, toasts, status polling)
  assets/            ascii-magic-*.png/.jpg (unused, safe to delete)

Dockerfile           Multi-stage (builder= rust:1.89-slim, runtime=debian:bookworm-slim)
docker-compose.yml        Local stack
docker-compose.prod.yml   Production stack (healthchecks, passwords, restart, non-root)

scripts/verify.js    Integration smoke test (9 checks)
scripts/start-local.ps1  Windows one-step start
```

---

## 16. What We Would Add Next

In priority order if the product were to grow:

1. **Durable analytics queue (Redis Streams / Kafka / SQS)** — bounded consumer, no `INSERT` per visit.
2. **Read replicas + CDN** — redirect replicas behind every region; edge-cache the 302 mapping.
3. **Sharding** — `hash(short_code) % N` PostgreSQL shards; new-URL creation rules stay on the primary.
4. **Custom aliases + expiring URLs** — only after auth is in place.
5. **Abuse/reputation checks** on `longUrl` (blocklist, entropy heuristics).
6. **Managed backups** of `postgres_data` and `click_events` outbox (already in `README.md`'s callout).
