-- 看板的「近一小時讀碼失敗率」與前科查詢以 started_ms 篩選；沒有索引時每次都把整張包裹表掃一遍
-- （看板每 10 秒、每送出一筆回報都會查）
CREATE INDEX IF NOT EXISTS idx_parcels_started_ms ON parcels(started_ms);
