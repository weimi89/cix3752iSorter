-- 讀碼站照片：讀碼器每件拍的圖經 FTP 上傳到本程式，縮成證據圖存在 data/images/YYYY-MM-DD/。
-- parcel_id NULL = 收到時對不到件（時間窗口內沒有剛綁條碼的包裹），照片仍留著可依時間翻。
CREATE TABLE parcel_images (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    parcel_id   INTEGER REFERENCES parcels(id) ON DELETE SET NULL,
    file_name   TEXT    NOT NULL,                  -- 讀碼器上傳時的檔名（含拍照時間、觸發序號）
    rel_path    TEXT    NOT NULL,                  -- 相對 data/images 的路徑
    orig_path   TEXT,                              -- 保留原圖時的相對路徑（讀碼失敗件）
    size        INTEGER NOT NULL,                  -- 證據圖大小（bytes）
    orig_size   INTEGER NOT NULL,                  -- 上傳原圖大小（bytes）
    received_ms INTEGER NOT NULL,
    received_at TEXT    NOT NULL                   -- 本機時間 'YYYY-MM-DD HH:MM:SS.mmm'
);
CREATE INDEX idx_parcel_images_parcel   ON parcel_images(parcel_id);
CREATE INDEX idx_parcel_images_received ON parcel_images(received_at);
