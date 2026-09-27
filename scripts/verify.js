#!/usr/bin/env node
/**
 * Shorty smoke test — run this after starting the app to confirm everything works.
 *
 * Usage:
 *   npm run verify                         # checks http://localhost:3000
 *   npm run verify -- https://your-app.up.railway.app
 *
 * No Jenkins, no extra tools — just Node (18+).
 */

const BASE_URL = (process.argv[2] || process.env.BASE_URL || 'http://localhost:3000').replace(/\/$/, '')
const STARTUP_TIMEOUT_MS = Number.parseInt(process.env.VERIFY_TIMEOUT_MS, 10) || 120000
const POLL_INTERVAL_MS = 2000

const results = []

function pass(name, detail) {
  results.push({ name, ok: true, detail })
  console.log(`  ✓ ${name}${detail ? ` — ${detail}` : ''}`)
}

function fail(name, detail) {
  results.push({ name, ok: false, detail })
  console.error(`  ✗ ${name}${detail ? ` — ${detail}` : ''}`)
}

async function waitForServer() {
  const deadline = Date.now() + STARTUP_TIMEOUT_MS
  let lastError = 'server did not respond'

  process.stdout.write(`Waiting for ${BASE_URL} (up to ${STARTUP_TIMEOUT_MS / 1000}s)...`)

  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${BASE_URL}/health`)
      if (response.ok) {
        process.stdout.write(' ready\n')
        return
      }
      lastError = `health returned ${response.status}`
    } catch (error) {
      lastError = error.message
    }
    process.stdout.write('.')
    await sleep(POLL_INTERVAL_MS)
  }

  process.stdout.write('\n')
  throw new Error(lastError)
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function checkHealth() {
  const response = await fetch(`${BASE_URL}/health`)
  if (!response.ok) {
    fail('GET /health', `status ${response.status}`)
    return null
  }

  const health = await response.json()

  if (health.status !== 'ok') {
    fail('GET /health', `status is "${health.status}" (expected "ok")`)
    return null
  }

  if (health.database !== 'connected') {
    fail('GET /health', `database is "${health.database}"`)
    return null
  }

  if (health.redis !== 'connected') {
    fail('GET /health', `redis is "${health.redis}"`)
    return null
  }

  pass('GET /health', 'PostgreSQL and Redis connected')
  return health
}

async function checkFrontend() {
  const response = await fetch(`${BASE_URL}/`)
  if (!response.ok) {
    fail('GET / (frontend)', `status ${response.status}`)
    return
  }

  const html = await response.text()
  if (!html.includes('Hush') || !html.includes('shortenForm')) {
    fail('GET / (frontend)', 'page HTML looks wrong')
    return
  }

  pass('GET / (frontend)', 'home page loads')

  for (const asset of ['/shorty.css', '/app.js']) {
    const assetResponse = await fetch(`${BASE_URL}${asset}`)
    if (!assetResponse.ok) {
      fail(`GET ${asset}`, `status ${assetResponse.status}`)
    } else {
      pass(`GET ${asset}`, 'ok')
    }
  }
}

async function checkShortenAndRedirect() {
  // Use a unique URL each run so we always exercise the full create path.
  const longUrl = `https://example.com/shorty-verify-${Date.now()}`

  const shortenResponse = await fetch(`${BASE_URL}/shorten`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ longUrl }),
  })

  if (!shortenResponse.ok) {
    const body = await shortenResponse.text()
    fail('POST /shorten', `status ${shortenResponse.status}: ${body}`)
    return null
  }

  const first = await shortenResponse.json()
  if (!first.shortCode || !first.shortUrl) {
    fail('POST /shorten', 'response missing shortCode or shortUrl')
    return null
  }

  pass('POST /shorten', `created code "${first.shortCode}"`)

  // Idempotency — same URL must return the same short code.
  const retryResponse = await fetch(`${BASE_URL}/shorten`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ longUrl }),
  })

  const second = await retryResponse.json()
  if (second.shortCode !== first.shortCode) {
    fail('POST /shorten (idempotency)', `got "${second.shortCode}", expected "${first.shortCode}"`)
  } else {
    pass('POST /shorten (idempotency)', 'same URL returns same code')
  }

  const redirectResponse = await fetch(`${BASE_URL}/${first.shortCode}`, { redirect: 'manual' })
  if (redirectResponse.status !== 302) {
    fail(`GET /${first.shortCode}`, `expected 302, got ${redirectResponse.status}`)
  } else {
    const location = redirectResponse.headers.get('location')
    if (location !== longUrl) {
      fail(`GET /${first.shortCode}`, `Location header was "${location}"`)
    } else {
      pass(`GET /${first.shortCode}`, '302 redirect to original URL')
    }
  }

  const statsResponse = await fetch(`${BASE_URL}/stats/${first.shortCode}`)
  if (!statsResponse.ok) {
    fail(`GET /stats/${first.shortCode}`, `status ${statsResponse.status}`)
    return first
  }

  const stats = await statsResponse.json()
  if (stats.longUrl !== longUrl) {
    fail(`GET /stats/${first.shortCode}`, 'longUrl mismatch')
  } else {
    pass(`GET /stats/${first.shortCode}`, `destination and ${stats.clickCount} click(s)`)
  }

  return first
}

async function checkValidation() {
  const badResponse = await fetch(`${BASE_URL}/shorten`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ longUrl: 'not-a-valid-url' }),
  })

  if (badResponse.status === 400) {
    pass('POST /shorten (validation)', 'rejects bad URLs with 400')
  } else {
    fail('POST /shorten (validation)', `expected 400, got ${badResponse.status}`)
  }
}

async function main() {
  console.log(`\nShorty verify — ${BASE_URL}\n`)

  try {
    await waitForServer()
  } catch (error) {
    console.error(`\nCould not reach the server: ${error.message}`)
    console.error('\nStart it first:')
    console.error('  docker compose up -d --build')
    console.error('  npm run verify\n')
    process.exit(1)
  }

  await checkHealth()
  await checkFrontend()
  await checkShortenAndRedirect()
  await checkValidation()

  const failed = results.filter((r) => !r.ok)
  console.log('')

  if (failed.length === 0) {
    console.log(`All ${results.length} checks passed. Shorty is fully working.\n`)
    process.exit(0)
  }

  console.error(`${failed.length} of ${results.length} checks failed.\n`)
  process.exit(1)
}

main().catch((error) => {
  console.error('\nVerify script crashed:', error.message)
  process.exit(1)
})
