-- 格口滿袋／換袋管理：每個格口一次一袋，落格件數累計到上限就提醒換袋（0 = 不限、不提醒）。
-- 2026-09-18 有 85 件「已完成卻又進線」，多半是袋滿彈回皮帶；系統之前不知道袋子裝了幾件。
ALTER TABLE chutes ADD COLUMN bag_limit INTEGER NOT NULL DEFAULT 0;

-- 每一袋一列：ended_ms NULL = 現在這袋。本袋件數不存在這裡，開著時從 parcels 算
-- （chute_code 相同、status=3、ended_ms >= started_ms），關袋時才把 count 定版，重啟不會算錯。
CREATE TABLE chute_bags (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    chute_code TEXT    NOT NULL,
    seq        INTEGER NOT NULL,                  -- 當天第幾袋（每天從 1 起算）
    started_ms INTEGER NOT NULL,
    ended_ms   INTEGER,
    count      INTEGER NOT NULL DEFAULT 0,        -- 關袋時定版的件數
    closed_by  TEXT                               -- 'web' / 'desktop' / 'day' …
);
CREATE INDEX idx_chute_bags_open ON chute_bags(chute_code, ended_ms);
CREATE INDEX idx_parcels_chute_ended ON parcels(chute_code, ended_ms);
