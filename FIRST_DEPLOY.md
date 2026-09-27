# Hush — First Deploy, Click by Click

Zero Docker experience assumed. Every step says exactly where to click and what to type.
Part A runs the app on your own machine. Part B puts it on the internet for free.

> Nothing here costs money. You will create accounts on Render, Neon, and
> Upstash (all free tiers, no credit card), plus GitHub if you lack one.

---

# PART A — Run it locally with Docker

## A1. Install Docker Desktop

1. Open your browser, go to **docker.com** → click **Download Docker Desktop** → choose **Windows**.
2. Run the downloaded installer. Keep **"Use WSL 2 instead of Hyper-V"** checked. Click through.
3. When it asks, restart your computer.
4. After restart, open **PowerShell as Administrator** (Start menu → type `PowerShell` → right-click → Run as administrator) and type:
   ```powershell
   wsl --install
   ```
   If it says WSL is already installed, that's fine.
5. Open **Docker Desktop** from the Start menu. Wait until the whale icon in the
   bottom-right system tray **stops animating** (about a minute on first run).
   If it asks to accept terms or sign in, accept terms; sign-in is optional.

## A2. Install Node.js and Git

1. Go to **nodejs.org** → download **LTS** → install with all defaults.
2. Go to **git-scm.com** → download → install with all defaults.
3. Open a **new** PowerShell window (new, so it picks up the installs) and run:
   ```powershell
   node -v
   git --version
   docker ps
   ```
   All three must print versions / an empty table — **no errors**.
   If `docker ps` errors, Docker Desktop is still starting. Wait 30 seconds, retry.

## A3. Open the project folder

In PowerShell:

```powershell
cd "Z:\Projects\Better Projects\URL Shortner - caching\url-shortener"
```

You do **not** need to create or edit any `.env` file for local Docker —
`docker-compose.yml` already passes the right hostnames to the app.

## A4. Build and start (one command)

```powershell
docker compose up -d --build
```

What happens: Docker builds the Rust app (first time takes several minutes —
that is normal, it's compiling), then starts three containers: PostgreSQL,
Redis, and Hush. `-d` means "run in background."

## A5. Confirm everything started

```powershell
docker compose ps
```

All three rows must say **Up**. Then:

```powershell
docker compose logs app --tail=5
```

The last line must say `Shorty listening on http://0.0.0.0:3000`.
(Internal log name is still "Shorty" — the product is Hush. Harmless.)

## A6. Run the automated checks

```powershell
node scripts/verify.js
```

You want **all 9 checks green** (health, homepage, CSS, JS, shorten,
idempotency, 302 redirect, stats, validation). **Stop here if anything fails**
and jump to Part C troubleshooting. Do not deploy a red build.

## A7. Click through the app yourself

1. Browser → `http://localhost:3000`.
2. Paste `https://example.com/a/very/long/path` → click **Shorten** → **Copy**.
3. Open the short link in a new tab → you must land on example.com.
4. Paste just the **code** (e.g. `b`) into **Inspect** → Lookup → destination + click count.
   (Clicks flush every 5 seconds — wait a moment, Lookup again, count goes up.)
5. Paste a **full short URL** (`http://localhost:3000/b`) into Inspect → must still resolve.

## A8. Prove the safety nets work

Rate limit — paste into PowerShell (11 rapid requests; first 10 pass, 11th is rejected):

```powershell
1..11 | ForEach-Object {
  (Invoke-WebRequest -Method Post -Uri http://localhost:3000/shorten `
    -ContentType "application/json" `
    -Body '{"longUrl":"https://example.com/rl-test"}' `
    -SkipHttpErrorCheck).StatusCode
}
```

Redis fallback — stop Redis, app must survive on Postgres alone:

```powershell
docker compose stop redis
# now shorten + redirect in the browser — both must still work
docker compose start redis
```

## A9. Everyday commands (bookmark these)

```powershell
docker compose stop          # pause everything, data is kept
docker compose start         # resume
docker compose down          # stop + remove containers, data is kept
docker compose logs -f app   # live app logs, Ctrl+C to exit
```

⚠️ `docker compose down -v` **deletes the database**. Only for a deliberate fresh start.

---

# PART B — Free live deploy (Render + Neon + Upstash)

Result: `https://hush.abiram08.dev`, TLS included, $0/month.
Trade-off to know upfront: free Render sleeps after 15 min idle, so the first
visit after a quiet spell takes ~30–60s. Neon sleeps too (adds a few seconds).
Fine for a portfolio project.

## B1. Push the code to GitHub

Render deploys from GitHub. Your folder is not a repo yet, so (in PowerShell,
inside the `url-shortener` folder):

```powershell
git init -b main
git add .
git status
```

Check the list: **`.env` must NOT appear** (it's gitignored — secrets never go to GitHub).
Then:

```powershell
git commit -m "Hush — ad-free URL shortener"
```

Browser → **github.com** → **New repository** → name it `hush`, leave everything
unchecked → **Create**. Back in PowerShell (replace `<you>`):

```powershell
git remote add origin https://github.com/<you>/hush.git
git push -u origin main
```

Refresh GitHub — your files are there.

## B2. Create the free Postgres (Neon — no expiry)

1. Browser → **neon.com** → Sign up (GitHub login works).
2. **New Project** → name `hush` → pick the region closest to you → Create.
3. Copy the **connection string** (toggle "Pooled" if offered). It looks like:
   `postgresql://user:pass@ep-xxx.aws.neon.tech/hush?sslmode=require`
4. Paste it into a temporary Notepad file. This becomes `DATABASE_URL`.
   (Why Neon and not Render's own free Postgres: Render's free database
   **expires after 30 days**. Neon's free tier does not expire.)

## B3. Create the free Redis (Upstash — no card)

1. Browser → **upstash.com** → Sign up → **Create Database** → name `hush-cache`,
   nearest region, TLS **on**.
2. On the database page find **Redis URL** in `rediss://` form (NOT the REST URL):
   `rediss://default:<password>@<host>:6379`
3. Paste it into the same Notepad file. This becomes `REDIS_URL`.

## B4. Deploy the app (Render — Docker, free)

1. Browser → **render.com** → Sign up with GitHub.
2. Click **New +** (top right) → **Web Service** → **Connect** your `hush` repo.
3. Settings page:
   - **Environment:** Docker (auto-detected from your Dockerfile — leave it)
   - **Instance type:** Free
   - **Health check path:** type `/health`
   - Click **Deploy** (it will fail to start properly until B5 — that's expected).
4. While it builds, generate a one-time salt for short codes. In PowerShell:
   ```powershell
   python -c "import random;print(hex(random.getrandbits(64)))"
   ```
   Copy the output (e.g. `0x9f2c…`). Set once, never change it.

## B5. Add the environment variables (Render Dashboard)

Go to your service → **Environment** (left sidebar) → add each key:

| Key | Value — where from |
|---|---|
| `PORT` | `10000` |
| `BASE_URL` | your Render URL for now: `https://<your-svc>.onrender.com` (you'll change it in B6) |
| `DATABASE_URL` | Neon string from B2 |
| `REDIS_URL` | Upstash `rediss://` string from B3 |
| `DATABASE_MAX_CONNECTIONS` | `5` |
| `DATABASE_ACQUIRE_TIMEOUT_SECONDS` | `5` |
| `TRUSTED_PROXY_COUNT` | `1` (Render sits in front of the app — without this, rate limits see the proxy's IP, not the visitor's) |
| `SHORT_CODE_SALT` | the hex from B4 |
| `CACHE_TTL_SECONDS` | `86400` |
| `RATE_LIMIT_MAX_REQUESTS` | `10` |
| `RATE_LIMIT_WINDOW_MS` | `60000` |
| `ANALYTICS_BATCH_SIZE` | `1000` |
| `ANALYTICS_FLUSH_INTERVAL_SECONDS` | `5` |
| `IDEMPOTENCY_KEY_TTL_SECONDS` | `3600` |

Click **Save** — Render redeploys automatically.

## B6. Verify the live app (same 9 checks, remote)

1. In Render, open **Logs**. Wait for `Shorty listening on http://0.0.0.0:10000`
   with no retry panics.
2. Browser → `https://<your-svc>.onrender.com/health` → must say
   `"status":"ok"` with redis + database `connected`.
3. In PowerShell:
   ```powershell
   node scripts/verify.js https://<your-svc>.onrender.com
   ```
   All 9 green. Then the live rate-limit proof — same loop as A8 but remote
   (first request may take ~60s cold start; that's the free tier waking up):
   ```powershell
   1..11 | ForEach-Object {
     (Invoke-WebRequest -Method Post -Uri https://<your-svc>.onrender.com/shorten `
       -ContentType "application/json" `
       -Body '{"longUrl":"https://example.com/rl-live"}' `
       -SkipHttpErrorCheck).StatusCode
   }
   ```
   Expect ten `201`s then a `429`. (This also proves the Lua limiter works on
   Upstash — if the 11th is a `500` instead, stop and report it; there's a
   non-Lua fallback ready.)

## B7. Point `hush.abiram08.dev` at it

1. Render → your service → **Settings → Custom Domains → Add**
   → type `hush.abiram08.dev`. Render shows a DNS target like
   `<your-svc>.onrender.com`.
2. Open wherever `abiram08.dev` DNS lives (registrar / Cloudflare / Vercel):
   add record → Type `CNAME`, host/name `hush`, value `<your-svc>.onrender.com`. Save.
3. Back in Render, wait for **Certificate: Issued** (auto TLS, a few minutes).
4. In Render **Environment**, change `BASE_URL` to `https://hush.abiram08.dev`
   → Save (redeploys).
5. Final proof:
   ```powershell
   node scripts/verify.js https://hush.abiram08.dev
   ```
6. Portfolio: add a Hush project card on `abiram08.dev` linking to it.

## B8. Free monitoring (your prod alarm)

1. **uptimerobot.com** → free account → **Add Monitor** → type HTTP(s),
   URL `https://hush.abiram08.dev/health`, interval 5 min,
   keyword alert for `"status":"ok"`.
2. You'll get emailed if the app, Postgres, or Redis goes down.

## B9. Later, if it outgrows free

- Sleep/cold starts annoy you → Render **Starter $7/mo** (always-on, same service, one click, no redeploy).
- More traffic → Neon Launch (pay-as-you-go, no minimum) and Upstash pay-as-you-go.
- Nothing in the app needs to change — only the plan buttons.

---

# PART C — Troubleshooting (look here first when stuck)

| Symptom | Fix |
|---|---|
| `docker ps` errors / "cannot find the file specified" | Docker Desktop isn't running. Start it from the Start menu, wait for the tray whale to go still, retry. |
| `password authentication failed for user "postgres"` | Stale local volume with an old password. `docker compose down -v` (wipes local DB only) then `up -d --build` again. |
| `port is already allocated` / 3000 in use | Something else uses port 3000. Stop it, or change `ports: "3000:3000"` in `docker-compose.yml`. |
| Verify says "Could not reach the server" | `docker compose ps` — is app Up? Then `docker compose logs app --tail=20` and read the last lines. |
| Render build fails | Open the failed deploy → **Logs** → copy the last ~30 lines. Rust compile errors name the file + line. |
| Live `/health` says `degraded` | One backend unreachable: check Neon string has `?sslmode=require`; check Upstash URL starts with `rediss://` (double-s). |
| Live 11th request is `500`, not `429` | Lua may be blocked on the Redis host. Report it — fallback limiter is ready. |
| Domain shows Render "not found" page | DNS hasn't propagated (wait 10 min) or CNAME value mistyped. Check with `nslookup hush.abiram08.dev`. |
| Certificate stuck on "Pending" | CNAME wrong or still propagating. Render can't issue until the CNAME resolves. |

---

# What you do NOT need

- Jenkins, Kubernetes, a build server, SSH deploy scripts.
- A credit card (Render, Neon, Upstash free tiers all start without one).
- Changing any code — the Dockerfile reads `$PORT`, migrations run themselves
  at boot, and `.env` never leaves your machine.
