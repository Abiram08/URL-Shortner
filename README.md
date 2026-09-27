# Hush

Hush turns long URLs into short, shareable links. No ads, no login, no tracking — just the link. I rebuilt `Shorty` in Rust to learn how a real service combines PostgreSQL, Redis caching, rate limiting, and durable analytics with Axum + Tokio.

> **Live URL:** *(add your Railway URL here after deploy)*

## Features

- Shorten `http://` and `https://` URLs
- Base62 short codes backed by PostgreSQL `SERIAL` IDs (collision-free by construction)
- Redis cache-aside lookups with a PostgreSQL fallback; `302` redirects
- Redis-backed, **Lua-atomic** rate limiting for `POST /shorten` (shared across instances)
- Idempotent shortening — the same URL always returns the same short code (also `Idempotency-Key` header)
- One-viewport, ad-free single page: `Shorten` + `Inspect` side by side; no recent-links enumeration
- Single stylesheet (`shorty.css` — Vercel brand + page overrides merged)
- Automated tests (`cargo test` + `node scripts/verify.js`)
- Docker production stack with persistent volumes, restart policies, `HEALTHCHECK`, and a **non-root** runtime
- `/health` (probes both backends) and `/metrics` (Prometheus), graceful shutdown, and structured tracing
- Versioned database migrations (`sqlx::migrate!`)
- Durable click analytics via a PostgreSQL `click_events` outbox

## Run Locally

### Docker development stack

```bash
docker compose up -d --build
```

Open `http://localhost:3000`.

**Check everything works:**

```bash
node scripts/verify.js
```

Or on Windows, start Docker and verify in one step: `.\scripts\start-local.ps1`

See [DEPLOY.md](./DEPLOY.md) for the full deploy and verify guide (no Jenkins required).

### Local Rust process

Start PostgreSQL and Redis first, then:

```bash
cargo test
cargo run
```

Copy `.env.example` to `.env` and set the connection values for your machine. The development Compose file keeps PostgreSQL and Redis private to the Docker network; use `docker compose exec` if you need to inspect them.

## API

### Create a short URL

```bash
curl -X POST http://localhost:3000/shorten \
  -H "Content-Type: application/json" \
  -d '{"longUrl":"https://example.com/a/very/long/path"}'
```

```json
{
  "shortUrl": "http://localhost:3000/b",
  "shortCode": "b"
}
```

### Redirect

```bash
curl -i http://localhost:3000/b
```

Returns `302 Found` with a `Location` header.

### Stats

```bash
curl http://localhost:3000/stats/b
```

### Health

```bash
curl http://localhost:3000/health
```

The public app intentionally does not expose recent-link enumeration or cache-flush controls. Those would leak user URLs or allow anyone to disrupt the cache.

## Production Deployment

The included `docker-compose.prod.yml` runs Hush, PostgreSQL, and password-protected Redis with restart policies, healthchecks, persistent volumes, and a non-root app container.

1. Copy `.env.example` to `.env` on the server.
2. Set strong unique values for `POSTGRES_PASSWORD` and `REDIS_PASSWORD`.
3. Set `BASE_URL` to the public HTTPS URL, for example `https://hush.example.com`.
4. Point your domain at the server and put HTTPS reverse proxy software such as Caddy or Nginx in front of port `3000`.
5. Start the stack:

```bash
docker compose -f docker-compose.prod.yml up -d --build
```

Check it:

```bash
curl https://hush.example.com/health
```

Scrape `/metrics` with Prometheus to monitor request rates, latencies, and status codes.

Do not expose PostgreSQL or Redis ports publicly. Back up the `postgres_data` volume. Analytics are durable (every click is a `click_events` row), flushed in batches every 5 seconds; at very high throughput you would move ingestion to Redis Streams or a dedicated queue.

## Performance

I load-tested `GET /:code` with `autocannon -c 100 -d 15`. The endpoint returns a `302 Found`, so autocannon counts the responses as non-2xx, but the latency and throughput numbers still measure the redirect path.

First I warmed a code in Redis to measure the cache-hit path, then I deleted that key so every request had to hit PostgreSQL.

| Lookup path | p50 latency | p99 latency | requests/sec |
| ----------- | ----------- | ----------- | ------------ |
| Cache hit (Redis) | 45 ms | 95 ms | 2102 |
| Cache miss (PostgreSQL only) | 44 ms | 71 ms | 2139 |

These numbers are lower than a pure in-memory queue because every redirect now writes a durable `click_events` row. That is the intentional trade-off: the app survives a crash without losing analytics, at the cost of a PostgreSQL write per redirect. At higher scale you would move that write to Redis Streams or a dedicated message queue.

## Scaling this further

If Hush ever outgrows a single instance, I'd shard PostgreSQL by hashing the short code across several database instances. Since a short code is unique and never changes, a hash of it routes every lookup to exactly one shard with no cross-shard joins, which keeps the redirect path a single query.

Redirects vastly outnumber new-URL creation in a real system, so I'd put read replicas in front of that traffic and keep writes on the primary. I'd also push redirects to a CDN or edge cache, because the short-code-to-long-URL mapping is stable after creation, which makes it a near-perfect edge-cacheable workload.

The analytics outbox already survives an app crash because every redirect writes to PostgreSQL, but a per-click INSERT has a ceiling. Once redirect volume outgrows what Postgres can absorb, I'd move ingestion to Redis Streams or a dedicated queue (Kafka/SQS) and keep a separate consumer that batches updates into the database. That keeps durability while removing the write pressure from the redirect path.

## Architecture

```text
Browser
  |
  +-- POST /shorten --> Rate limiter (Redis+Lua) --> PostgreSQL (SERIAL->Base62) --> Redis (SETEX)
  |
  +-- GET /:code ----> Redis cache --miss--> PostgreSQL
                              |
                              +--> 302 redirect (Location)
                              +--> click_events INSERT (durable) -> 5s batch UPDATE urls.click_count
  |
  +-- GET /stats/:code --> Redis or PostgreSQL
  +-- GET /health ------> Redis PING + SELECT 1   (HealthResponse { status, redis, database })
  +-- GET /metrics -----> Prometheus exposition
```

## Project Structure

```text
public/            Hush single page (index.html + shorty.css (merged) + app.js)
  index.html       Sticky nav + header ("Short links, no noise.") + 2-col split: Shorten / Inspect
  shorty.css       Vercel brand primitives (.vbg-*) + page-owned vbg-custom-* (one request)
  app.js           Fetch frontend (paste, validate, toasts, health polling)
  assets/          Unused ascii-magic-*.png/.jpg (safe to delete)

migrations/        sqlx versioned (0001_initial.sql, 0002_click_events.sql)
src/               Rust backend (Axum, PostgreSQL, Redis, rate limiter, analytics)
  main.rs          AppState, Router, health/shorten/redirect/stats handlers.
  config.rs        Typed config (env + truncation/XFF gates + pool tuning)
  db.rs            PgPool create + retry + initialize_database
  cache.rs         Redis MultiplexedConnection + retry + SETEX helpers
  shortener.rs     Base62, validators (short_code, long_url cap 2048)
  rate_limiter.rs  Redis+Lua fixed-window, per-IP, atomic
  idempotency.rs   Redis-backed key replay
  analytics.rs     click_events outbox + 5s flush
  metrics.rs       Prometheus recorder + route-normalized middleware

Dockerfile         Multi-stage (rust:1.89-slim -> debian:bookworm-slim, non-root shorty user)
docker-compose.yml        Local stack
docker-compose.prod.yml   Production stack
scripts/verify.js  Integration smoke test (9 checks)
```

## What I Would Add Next

- User accounts and private link management
- Durable analytics queue using Redis Streams or a message broker
- Expiring links and custom aliases
- Abuse protection and URL reputation checks
- Managed PostgreSQL and Redis backups
