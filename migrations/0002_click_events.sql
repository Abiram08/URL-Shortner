-- A durable outbox for click analytics.
-- Each redirect writes one row here; a background job aggregates them and
-- updates urls.click_count. If the app restarts, rows survive the crash.
CREATE TABLE IF NOT EXISTS click_events (
    id BIGSERIAL PRIMARY KEY,
    short_code VARCHAR(10) NOT NULL,
    clicked_at TIMESTAMP WITHOUT TIME ZONE DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_click_events_short_code ON click_events(short_code);
CREATE INDEX IF NOT EXISTS idx_click_events_id ON click_events(id);
