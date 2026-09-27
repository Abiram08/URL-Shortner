-- The first migration creates the core urls table and its indexes.
-- Using migrations instead of IF NOT EXISTS statements means every schema
-- change is versioned and reproducible across dev, staging, and production.
CREATE TABLE IF NOT EXISTS urls (
    id SERIAL PRIMARY KEY,
    short_code VARCHAR(10) UNIQUE NOT NULL,
    long_url TEXT NOT NULL,
    click_count INTEGER DEFAULT 0,
    created_at TIMESTAMP WITHOUT TIME ZONE DEFAULT NOW(),
    expires_at TIMESTAMP WITHOUT TIME ZONE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_urls_long_url ON urls(long_url);
CREATE UNIQUE INDEX IF NOT EXISTS idx_urls_short_code ON urls(short_code);
