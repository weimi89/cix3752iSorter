-- 拿掉格口滿袋／換袋管理：現場格口下是多台大籠車，沒有「換袋」這回事（2026-09-21 業主裁示）。
-- 0004 建的表與欄位在此收回；每袋紀錄沒有人用，直接丟掉。
DROP INDEX IF EXISTS idx_parcels_chute_ended;
DROP TABLE IF EXISTS chute_bags;
ALTER TABLE chutes DROP COLUMN bag_limit;
