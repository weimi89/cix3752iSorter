-- 照片另存條碼：包裹資料保留天數比照片短，包裹清掉後照片的包裹編號會被清空（ON DELETE SET NULL），
-- 沒有條碼就只能依時間翻，收件證據對不回單號。照片對上包裹時寫入、包裹條碼改了跟著改。
-- 讀碼失敗（'NoRead'）沒有單號可存，存空值，免得用佔位字一查就是成千張不相干的照片。
-- 既有已對上包裹的照片在這裡補上；已經失去包裹的照片補不回來
ALTER TABLE parcel_images ADD COLUMN barcode TEXT;
UPDATE parcel_images SET barcode = (SELECT NULLIF(p.barcode, 'NoRead') FROM parcels p WHERE p.id = parcel_images.parcel_id) WHERE parcel_id IS NOT NULL;
-- 只查「包裹已清除」的照片（依單號找收件證據），部分索引只收這些列
CREATE INDEX IF NOT EXISTS idx_parcel_images_orphan_barcode ON parcel_images(barcode) WHERE parcel_id IS NULL;
