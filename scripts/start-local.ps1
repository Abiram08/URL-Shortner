# Start Shorty locally with Docker and run the smoke test.
# Usage (from the url-shortener folder):
#   .\scripts\start-local.ps1

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $ProjectRoot

Write-Host "`nShorty — local start`n" -ForegroundColor Cyan

# Make sure Docker Desktop is running
docker info *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Host "Docker is not running. Open Docker Desktop, wait until it is ready, then run this script again.`n" -ForegroundColor Red
    exit 1
}

Write-Host "Building and starting containers..." -ForegroundColor Yellow
docker compose up -d --build
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "`nRunning smoke test..." -ForegroundColor Yellow
node scripts/verify.js
exit $LASTEXITCODE
