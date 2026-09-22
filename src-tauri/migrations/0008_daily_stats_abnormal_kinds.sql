-- 異常分三類看：分揀機異常（既有 abnormal，狀態 4–8）、仲介機回傳（有讀到碼但落到異常口）、讀碼失敗（NoRead 落到異常口）。
-- 三類互斥，優先序 分揀機 > 仲介機回傳 > 讀碼失敗；規則在 db/abnormal_kind.rs。
-- 舊日子的回填要知道異常口代碼（在設定檔，SQL 拿不到），由 abnormal_kind::backfill_daily_stats 在啟動時做一次。
-- 既有 noread／defaulted 欄位不動，口徑照舊。
ALTER TABLE daily_stats ADD COLUMN middleware    INTEGER NOT NULL DEFAULT 0;  -- 仲介機回傳且已落異常口
ALTER TABLE daily_stats ADD COLUMN noread_landed INTEGER NOT NULL DEFAULT 0;  -- 讀碼失敗且已落異常口
