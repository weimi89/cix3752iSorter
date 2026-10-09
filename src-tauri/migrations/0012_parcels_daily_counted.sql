-- 這件包裹是否已計入當日統計（daily_stats）。收尾時與終態同一個交易標記；
-- 重啟收尾補計還沒標記的件——包括已經結束、但在計入統計之前就關閉的那幾件（只看「沒有結束時間」會漏掉它們）。
-- 既有資料一律視為已計入（預設 1，SQLite 加有常數預設值的欄位不重寫整張表）；新增的件由程式明寫 0
ALTER TABLE parcels ADD COLUMN daily_counted INTEGER NOT NULL DEFAULT 1;
