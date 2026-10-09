-- 刪除包裹時 SQLite 要逐件找出參照它的子表列（ON DELETE SET NULL）；子表的 parcel_id 沒有索引，
-- 每刪一件就把整張子表掃一遍，保留清理一次刪一天會長時間握住寫鎖，其他寫入等不到就失敗
CREATE INDEX IF NOT EXISTS idx_report_queue_parcel ON report_queue(parcel_id);
CREATE INDEX IF NOT EXISTS idx_print_jobs_parcel ON print_jobs(parcel_id);
CREATE INDEX IF NOT EXISTS idx_jam_events_parcel ON jam_events(parcel_id);
-- 入列以 response_id 判斷是否已有這筆回報；沒有索引時每次入列都掃整張回報表
CREATE INDEX IF NOT EXISTS idx_report_queue_response ON report_queue(response_id);
-- 每張讀碼站照片都要依時間找剛綁條碼的件；只靠 (parcel_id, ts_ms) 索引時是逐筆掃整張事件表。
-- 部分索引只收綁碼事件（每件一筆），寫入放大可忽略；查詢條件要寫字面值 source='camera' AND kind='bind' 才用得上
CREATE INDEX IF NOT EXISTS idx_parcel_events_bind_ts ON parcel_events(ts_ms) WHERE source = 'camera' AND kind = 'bind';
