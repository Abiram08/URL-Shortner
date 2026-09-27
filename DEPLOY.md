# Deploy & verify Shorty (no Jenkins needed)

Shorty does **not** use Jenkins. You do not need any CI/CD server to run or ship this project.

This guide covers:
1. Run locally with Docker
2. Check everything works with one command
3. Deploy live on Railway (optional, free tier friendly)

---

## 1. Run locally

**Requirements:** [Docker Desktop](https://www.docker.com/products/docker-desktop/) installed and running.

From the `url-shortener` folder:

```powershell
# Windows - one command: start + verify
.\scripts\start-local.ps1
```

Or step by step:

```bash
docker compose up -d --build
node scripts/verify.js
```

Open [http://localhost:3000](http://localhost:3000) in your browser.

Stop when done:

```bash
docker compose down
```

---

## 2. Verify everything works

The verify script hits every important endpoint and prints pass/fail:

| Check | What it proves |
| ----- | -------------- |
| `GET /health` | PostgreSQL + Redis are connected |
| `GET /` | Frontend HTML loads |
| `GET /shorty.css`, `/app.js` | Static assets work |
| `POST /shorten` | URL shortening works |
| Idempotency | Same URL -> same short code |
| `GET /:code` | Redirect returns 302 |
| `GET /stats/:code` | Stats endpoint works |
| `GET /metrics` | Prometheus metrics are exposed |
| Bad URL validation | Server rejects invalid input |
| Idempotency key | Same key returns same short code |

```bash
# Local
node scripts/verify.js

# Live site (after Railway deploy)
node scripts/verify.js https://your-app.up.railway.app
```

Also run unit tests anytime:

```bash
cargo test
```

**Recommended before you share or submit the project:**

```bash
cargo test && node scripts/verify.js
```

---

## 3. Deploy live on Railway (optional)

Railway builds from your Dockerfile - no Jenkins, no server to maintain.

### Step A - push to GitHub

1. Create a repo on GitHub
2. Push this project (`.env` is gitignored - never commit secrets)

### Step B - Railway app

1. Go to [railway.app](https://railway.app) and sign in with GitHub
2. **New Project > Deploy from GitHub repo** > pick your repo
3. Railway detects the `Dockerfile` automatically

### Step C - add PostgreSQL

1. In your Railway project: **+ New > Database > PostgreSQL**
2. Railway injects `DATABASE_URL` into your app

### Step D - add Redis (Upstash)

1. Go to [upstash.com](https://upstash.com) and create a free Redis database
2. Copy the **Redis URL** (starts with `rediss://` or `redis://`)
3. In Railway > your app > **Variables** > add:

| Variable | Value |
| -------- | ----- |
| `REDIS_URL` | paste from Upstash |
| `BASE_URL` | your Railway public URL, e.g. `https://shorty-production.up.railway.app` |
| `CACHE_TTL_SECONDS` | `86400` |
| `RATE_LIMIT_MAX_REQUESTS` | `10` |
| `RATE_LIMIT_WINDOW_MS` | `60000` |
| `ANALYTICS_BATCH_SIZE` | `1000` |
| `ANALYTICS_FLUSH_INTERVAL_SECONDS` | `5` |
| `IDEMPOTENCY_KEY_TTL_SECONDS` | `3600` |

Railway sets `PORT` and `DATABASE_URL` for you.

### Step E - confirm it works

After deploy finishes:

```bash
node scripts/verify.js https://YOUR-RAILWAY-URL.up.railway.app
```

All checks should pass. Add the live URL to the top of `README.md`.

You can also scrape `https://YOUR-RAILWAY-URL.up.railway.app/metrics` with Prometheus.

---

## Troubleshooting

| Problem | Fix |
| ------- | --- |
| `Docker is not running` | Open Docker Desktop, wait until green, retry |
| `password authentication failed` | Use `docker compose up` - do not run `cargo run` without matching Postgres |
| Health shows `degraded` | Wait 30s for containers to start, run `docker compose ps` |
| Verify times out | Run `docker compose logs app` and check for errors |

---

## What you do NOT need

- Jenkins
- Kubernetes
- A separate build server
- Manual SSH deploy scripts (for learning - Docker + Railway is enough)
