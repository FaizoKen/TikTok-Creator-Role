-- When this link's held list write is next re-sent; NULL when nothing is held.
--
-- RoleLogic holds a write that would remove many members (or empty the link)
-- until the server owner confirms it, and a confirmation lets the next write
-- through for an hour. So the config sync worker re-sends a held write every
-- 20 minutes. That schedule used to live only in the worker's memory: after a
-- restart nothing re-sent the list, and a confirmation in the dashboard waited
-- for a write that never came. The worker books it here too and picks it up
-- again when it starts.
ALTER TABLE role_links ADD COLUMN IF NOT EXISTS held_recheck_at TIMESTAMPTZ;
