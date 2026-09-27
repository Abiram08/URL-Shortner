/* Hush frontend — ad-free, no login, no tracking. */

function $(id) {
  return document.getElementById(id)
}

function setError(element, message) {
  element.textContent = message
  element.removeAttribute('hidden')
}

function clearError(element) {
  element.textContent = ''
  element.setAttribute('hidden', '')
}

async function withButtonBusy(button, loadingText, asyncWork) {
  // New markup nests the text in .btn-label + a .btn-spinner shown via :disabled.
  // Fall back to textContent for plain buttons (e.g. dashed Paste).
  var label = button.querySelector ? button.querySelector('.btn-label') : null
  var originalText = label ? label.textContent : button.textContent
  button.disabled = true
  if (label) label.textContent = loadingText
  else button.textContent = loadingText
  try {
    return await asyncWork()
  } finally {
    button.disabled = false
    if (label) label.textContent = originalText
    else button.textContent = originalText
  }
}

function isValidHttpUrl(value) {
  try {
    var parsedUrl = new URL(value)
    return parsedUrl.protocol === 'http:' || parsedUrl.protocol === 'https:'
  } catch (error) {
    return false
  }
}

// Accept either a bare code ("b") or a full short URL pasted in
// ("http://host/b", "host/b?x=1") — always return just the code.
function extractShortCode(raw) {
  var value = (raw || '').trim()
  if (!value) return ''
  // Strip query string and fragment first.
  value = value.split(/[?#]/)[0]
  // Take the last non-empty path segment.
  var parts = value.split('/').filter(function (p) { return p.length > 0 })
  var last = parts.length > 0 ? parts[parts.length - 1] : ''
  // A bare code has no dots/colons; a host does — keep the segment either way,
  // the server validates the format and answers 404 for unknown codes.
  return last || value
}

// Clipboard write with a legacy fallback for non-secure contexts
// (navigator.clipboard only exists on HTTPS and localhost).
async function copyTextWithFallback(text) {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch (error) {
    try {
      var area = document.createElement('textarea')
      area.value = text
      area.setAttribute('readonly', '')
      area.style.position = 'fixed'
      area.style.opacity = '0'
      document.body.appendChild(area)
      area.select()
      var ok = document.execCommand('copy')
      document.body.removeChild(area)
      return !!ok
    } catch (fallbackError) {
      return false
    }
  }
}

function showToast(message, tone) {
  var stack = $('toastStack')
  if (!stack) return
  var toast = document.createElement('div')
  toast.className = 'toast'
  if (tone) toast.setAttribute('data-tone', tone)
  toast.textContent = message
  stack.appendChild(toast)
  setTimeout(function () {
    toast.style.opacity = '0'
    toast.style.transform = 'translateY(4px)'
    toast.style.transition = 'opacity 180ms ease, transform 180ms ease'
    setTimeout(function () {
      if (toast.parentNode) toast.parentNode.removeChild(toast)
    }, 180)
  }, 2200)
}

// Shorten ----------------------------------------------------------------------
var shortenForm = $('shortenForm')
var shortenButton = $('shortenButton')
var shortenOutput = $('shortenOutput')
var shortUrlOutput = $('shortUrlOutput')
var shortCodeOutput = $('shortCodeOutput')
var shortUrlOrigin = $('shortUrlOrigin')
var shortenError = $('shortenError')
var openButton = $('openButton')
var inspectCreatedButton = $('inspectCreatedButton')
var copyButton = $('copyButton')
var copyButtonLabel = $('copyButtonLabel')
var longUrlInput = $('longUrl')

if (longUrlInput) {
  longUrlInput.addEventListener('input', function () {
    if (!shortenError.hasAttribute('hidden')) clearError(shortenError)
  })
}

shortenForm.addEventListener('submit', async function (event) {
  event.preventDefault()
  clearError(shortenError)
  shortenOutput.setAttribute('hidden', '')

  var longUrl = longUrlInput.value.trim()
  if (!longUrl) {
    setError(shortenError, 'Paste a URL first.')
    longUrlInput.focus()
    return
  }
  if (!isValidHttpUrl(longUrl)) {
    setError(shortenError, 'Use a valid http:// or https:// URL.')
    longUrlInput.focus()
    return
  }

  try {
    var response = await withButtonBusy(shortenButton, 'Shortening…', function () {
      return fetch('/shorten', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ longUrl: longUrl }),
      })
    })
    var data = await response.json()

    if (!response.ok) {
      setError(shortenError, data.error || 'Could not shorten that URL.')
      if (response.status === 429) showToast(data.error || 'Rate limited. Try again in a minute.', 'error')
      return
    }

    // Result state: mono selectable text, never cleared, never auto-navigated.
    // Clicking the URL copies it; Open ↗ handles navigation explicitly.
    shortUrlOutput.textContent = data.shortUrl
    shortUrlOutput.dataset.url = data.shortUrl
    shortUrlOutput.title = 'Click to copy'
    shortCodeOutput.textContent = data.shortCode
    if (shortUrlOrigin) {
      shortUrlOrigin.textContent = longUrl
      shortUrlOrigin.title = longUrl
    }
    if (openButton) openButton.href = data.shortUrl
    shortenOutput.removeAttribute('hidden')
    try { shortenOutput.scrollIntoView({ behavior: 'smooth', block: 'nearest' }) } catch (e) {}

    // prefill inspect so the user can check stats right away
    var statsCodeInput = $('statsCode')
    if (statsCodeInput) statsCodeInput.value = data.shortCode

    if (copyButtonLabel) copyButtonLabel.textContent = 'Copy'
    showToast('Short link created')
  } catch (error) {
    setError(shortenError, 'Cannot reach the server.')
    showToast('Cannot reach server', 'error')
  }
})

function copyShortUrl() {
  var url = shortUrlOutput.dataset.url || shortUrlOutput.textContent
  if (!url) return
  copyTextWithFallback(url).then(function (ok) {
    showToast(ok ? 'Copied to clipboard' : 'Copy failed — select the link manually', ok ? undefined : 'error')
  })
}

if (shortUrlOutput) {
  shortUrlOutput.addEventListener('click', copyShortUrl)
  shortUrlOutput.addEventListener('keydown', function (event) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      copyShortUrl()
    }
  })
}

if (copyButton) {
  copyButton.addEventListener('click', async function () {
    var text = shortUrlOutput.dataset.url || shortUrlOutput.textContent
    if (!text) return
    var ok = await copyTextWithFallback(text)
    if (ok) {
      if (copyButtonLabel) copyButtonLabel.textContent = 'Copied!'
      showToast('Copied to clipboard')
      setTimeout(function () {
        if (copyButtonLabel) copyButtonLabel.textContent = 'Copy'
      }, 1400)
    } else {
      showToast('Copy failed — select the link manually', 'error')
    }
  })
}

function setInspectParam(code) {
  // Shareable inspect state: ?inspect=b loads that code's stats on open.
  try {
    var url = new URL(window.location.href)
    if (code) url.searchParams.set('inspect', code)
    else url.searchParams.delete('inspect')
    history.replaceState(null, '', url.toString())
  } catch (error) {}
}

if (inspectCreatedButton) {
  inspectCreatedButton.addEventListener('click', function () {
    var code = shortCodeOutput.textContent.trim()
    if (!code) return
    $('statsCode').value = code
    setInspectParam(code)
    $('statsForm').dispatchEvent(new Event('submit', { cancelable: true }))
    try { $('statsResult').scrollIntoView({ behavior: 'smooth', block: 'nearest' }) } catch (e) {}
  })
}

var copyApiButton = $('copyApiButton')
if (copyApiButton) {
  copyApiButton.addEventListener('click', async function () {
    var snippet = document.querySelector('.api-snippet code')
    var text = snippet ? snippet.innerText : ''
    if (!text) return
    var ok = await copyTextWithFallback(text)
    showToast(ok ? 'Copied curl snippet' : 'Copy failed', ok ? undefined : 'error')
  })
}

// Inspect ----------------------------------------------------------------------
var statsForm = $('statsForm')
var statsButton = $('statsButton')
var statsResult = $('statsResult')
var statsError = $('statsError')

statsForm.addEventListener('submit', async function (event) {
  event.preventDefault()
  clearError(statsError)
  statsResult.setAttribute('hidden', '')

  var code = extractShortCode($('statsCode').value)
  if (!code) {
    setError(statsError, 'Enter a short code.')
    return
  }
  // Normalize the field so the user sees what was actually looked up.
  $('statsCode').value = code

  try {
    var response = await withButtonBusy(statsButton, 'Looking…', function () {
      return fetch('/stats/' + encodeURIComponent(code))
    })
    var data = await response.json()

    if (!response.ok) {
      setError(statsError, data.error || 'No link found for that code.')
      return
    }

    setInspectParam(data.shortCode)
    $('statsShortCode').textContent = data.shortCode
    $('statsLongUrl').textContent = data.longUrl
    $('statsClickCount').textContent = String(data.clickCount)
    $('statsCreatedAt').textContent = new Date(data.createdAt).toLocaleDateString()
    var statsOpenLink = $('statsOpenLink')
    if (statsOpenLink) {
      try {
        statsOpenLink.href = new URL(data.shortCode, window.location.origin + '/').toString()
      } catch (e) {
        statsOpenLink.href = '/' + data.shortCode
      }
    }
    statsResult.removeAttribute('hidden')
  } catch (error) {
    setError(statsError, 'Cannot reach the server.')
    showToast('Cannot reach server', 'error')
  }
})

// Status -----------------------------------------------------------------------
var statusBadge = $('statusBadge')

async function refreshStatus() {
  // #statusBadge is the pill; the visible text lives in .topbar-status-text
  // so updating it must not wipe the status dot.
  try {
    var response = await fetch('/health')
    var health = await response.json()
    var label = statusBadge.querySelector
      ? statusBadge.querySelector('.topbar-status-text')
      : null
    if (health.status === 'ok') {
      if (label) label.textContent = 'Operational'
      statusBadge.setAttribute('data-status', 'ok')
    } else {
      if (label) label.textContent = 'Degraded'
      statusBadge.setAttribute('data-status', 'degraded')
    }
  } catch (error) {
    // Don't show backend errors to users — keep last known state, silently retry.
  }
}

// Focus the main input on desktop so the first keystroke starts a shorten.
try {
  if (longUrlInput && !longUrlInput.value
      && window.matchMedia && window.matchMedia('(pointer:fine)').matches) {
    longUrlInput.focus({ preventScroll: true })
  }
} catch (error) {}

// Deep link: ?inspect=b pre-fills and auto-loads that code's stats.
try {
  var deepCode = extractShortCode(new URLSearchParams(window.location.search).get('inspect'))
  if (deepCode) {
    $('statsCode').value = deepCode
    statsForm.dispatchEvent(new Event('submit', { cancelable: true }))
  }
} catch (error) {}

refreshStatus()
setInterval(refreshStatus, 30000)
