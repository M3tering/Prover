-- Only deploy this schema into the new node's dedicated database.
ALTER TABLE m3ter_payloads ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'streamr';
ALTER TABLE m3ter_payloads ADD COLUMN IF NOT EXISTS queue_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE m3ter_payloads ADD COLUMN IF NOT EXISTS checked_at TIMESTAMPTZ;
ALTER TABLE m3ter_payloads ADD COLUMN IF NOT EXISTS reason TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS unique_meter_nonce ON m3ter_payloads(m3ter_id,nonce);
CREATE INDEX IF NOT EXISTS pending_actions ON m3ter_payloads(checked_at NULLS FIRST,m3ter_id,nonce) WHERE NOT is_verified;
CREATE TABLE IF NOT EXISTS node_identity (singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton), chain_id TEXT NOT NULL, rollup TEXT NOT NULL);
