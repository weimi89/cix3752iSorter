-- 統計「為什麼走異常口」要有代碼可分組：原因原本只寫在 event_log 的文字裡，統計不到。
-- NULL = 中介機正常給格口。代碼：NOREAD／TIMEOUT／LATE（回覆太晚，已走預設口）／
-- STORE_CLOSED 等中介機錯誤碼（與中介機同一組字串）／CHUTE_DISABLED／CHUTE_UNKNOWN／
-- NO_CHANNEL／MW_UNREACHABLE／LABEL_FETCH_FAILED
ALTER TABLE parcels ADD COLUMN chute_reason TEXT;

-- 卡件事件：每次堵塞開始（同一件包裹只記第一個 ~k）一筆，統計依模組／時段的卡件次數用。
-- 事件記錄裡的「M4 卡件」是節流過的告警，不是每次都有，不能拿來算次數。
CREATE TABLE jam_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms      INTEGER NOT NULL,
    created_at TEXT    NOT NULL,                  -- 本機時間 'YYYY-MM-DD HH:MM:SS.mmm'
    cart       INTEGER NOT NULL,
    pos        INTEGER NOT NULL,                  -- ~k 的位置
    module     INTEGER NOT NULL,                  -- pos/10+1，對應現場的 M1～M8
    parcel_id  INTEGER REFERENCES parcels(id) ON DELETE SET NULL,
    barcode    TEXT,
    chute_code TEXT
);
CREATE INDEX idx_jam_events_created ON jam_events(created_at);

-- 既有資料回填：每件包裹第一個 ~k 就是它那次堵塞的開始，時間取 parcel_events 裡的訊號時間
INSERT INTO jam_events (ts_ms, created_at, cart, pos, module, parcel_id, barcode, chute_code)
SELECT MIN(pe.ts_ms),
       strftime('%Y-%m-%d %H:%M:%f', MIN(pe.ts_ms) / 1000.0, 'unixepoch', 'localtime'),
       COALESCE(p.cart, -1), p.block_pos, p.block_pos / 10 + 1, p.id, p.barcode, p.chute_code
  FROM parcels p
  JOIN parcel_events pe ON pe.parcel_id = p.id AND pe.source = 'sorter' AND pe.kind = 'k'
 WHERE p.block_pos IS NOT NULL
 GROUP BY p.id;
