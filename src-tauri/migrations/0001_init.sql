-- 包裹主檔：皮帶 ~P 觸發時即建立，之後只更新；重啟不丟在途件
CREATE TABLE parcels (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    ulid          TEXT    NOT NULL UNIQUE,           -- 對外識別（回報中介機、日誌）
    barcode       TEXT    NOT NULL DEFAULT 'NoRead',
    chute_code    TEXT,                              -- L1…R5 / LS / RS / NG
    chute_cid     INTEGER,                           -- 下給分揀機的 CID
    chute_source  TEXT    NOT NULL DEFAULT 'pending',-- pending / api / default / noread / timeout / manual
    status        INTEGER NOT NULL DEFAULT 1,        -- 1初始化 2收件 3完成 4失去追蹤 5堵塞 6堵塞後取走 7指令取消 8觸發異常
    belt_slot     INTEGER,
    cart          INTEGER,
    ir_length     INTEGER,
    gap           INTEGER,
    block_pos     INTEGER,                           -- ~k 位置
    lost_pos      INTEGER,                           -- ~u 位置
    response_id   INTEGER,                           -- 中介機 /api/parcel 回的列印記錄 ID
    started_at    TEXT    NOT NULL,                  -- 本機時間 'YYYY-MM-DD HH:MM:SS.mmm'
    started_ms    INTEGER NOT NULL,                  -- 同上，epoch 毫秒（計算用）
    ended_ms      INTEGER,                           -- 終態時間；NULL = 在途
    travel_ms     INTEGER,                           -- ended_ms - started_ms
    updated_ms    INTEGER NOT NULL
);
CREATE INDEX idx_parcels_started_at ON parcels(started_at);
CREATE INDEX idx_parcels_barcode    ON parcels(barcode);
CREATE INDEX idx_parcels_chute_code ON parcels(chute_code);
CREATE INDEX idx_parcels_status     ON parcels(status);
CREATE INDEX idx_parcels_open       ON parcels(ended_ms) WHERE ended_ms IS NULL;

-- 訊號時間軸：每件包裹經過的每個訊號／指令／決策一筆
CREATE TABLE parcel_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    parcel_id  INTEGER NOT NULL REFERENCES parcels(id) ON DELETE CASCADE,
    ts_ms      INTEGER NOT NULL,
    source     TEXT    NOT NULL,   -- belt / sorter / camera / api / tracker / printer
    kind       TEXT    NOT NULL,   -- P / L / O / E / bind / chute / Kn / c / j / g / e / k / u / Kx / print …
    raw        TEXT                -- 原始行或說明
);
CREATE INDEX idx_parcel_events_parcel ON parcel_events(parcel_id, ts_ms);

-- 列印佇列：任務落地，重啟不丟
CREATE TABLE print_jobs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    parcel_id     INTEGER REFERENCES parcels(id) ON DELETE SET NULL,
    barcode       TEXT    NOT NULL,
    chute_code    TEXT    NOT NULL,
    printer_port  TEXT    NOT NULL,                  -- USB bus-port，如 1-3.1
    profile       TEXT,                              -- PAPER-01#100*150
    tspl_path     TEXT    NOT NULL,                  -- 已轉好的 TSPL 檔（data/print/…）
    status        TEXT    NOT NULL DEFAULT 'pending',-- pending / printing / done / failed
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    not_before_ms INTEGER NOT NULL DEFAULT 0,        -- 送印延遲（格口號-1）*400ms
    created_ms    INTEGER NOT NULL,
    finished_ms   INTEGER
);
CREATE INDEX idx_print_jobs_pending ON print_jobs(printer_port, status, not_before_ms);

-- 中介機回報佇列（POST /api/report），指數退避
CREATE TABLE report_queue (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    parcel_id      INTEGER REFERENCES parcels(id) ON DELETE SET NULL,
    response_id    INTEGER NOT NULL,
    payload        TEXT    NOT NULL,                 -- JSON
    status         TEXT    NOT NULL DEFAULT 'pending',-- pending / sending / success / failed
    retry_count    INTEGER NOT NULL DEFAULT 0,
    next_retry_ms  INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT,
    created_ms     INTEGER NOT NULL,
    finished_ms    INTEGER
);
CREATE INDEX idx_report_queue_due ON report_queue(status, next_retry_ms);

-- 格口對照：代號 ↔ 分揀機 CID ↔ 印表機
CREATE TABLE chutes (
    code          TEXT    PRIMARY KEY,               -- L1…R5 / LS / RS / NG
    label         TEXT    NOT NULL DEFAULT '',
    cid           INTEGER NOT NULL,
    printer_port  TEXT,                              -- NULL = 該格口不印
    enabled       INTEGER NOT NULL DEFAULT 1,
    sort_order    INTEGER NOT NULL DEFAULT 0
);

-- 系統事件（照 cix3752iLabelPrint）
CREATE TABLE event_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    level      TEXT NOT NULL,      -- info / warn / error
    category   TEXT NOT NULL,      -- belt / sorter / camera / tracker / chute / middleware / printer / server
    action     TEXT NOT NULL,
    message    TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_event_log_created ON event_log(created_at);
CREATE INDEX idx_event_log_cat     ON event_log(category, level);

-- 每日件數快取（啟動時不必 count(*) 全表）
CREATE TABLE daily_stats (
    day        TEXT    PRIMARY KEY,                  -- YYYY-MM-DD
    total      INTEGER NOT NULL DEFAULT 0,
    done       INTEGER NOT NULL DEFAULT 0,
    noread     INTEGER NOT NULL DEFAULT 0,
    defaulted  INTEGER NOT NULL DEFAULT 0,           -- 走預設口
    abnormal   INTEGER NOT NULL DEFAULT 0            -- 狀態 4–8
);

-- 現場預設格口（以舊 chute 表為初值；正式機原始碼到手後校正）
INSERT INTO chutes (code, label, cid, printer_port, enabled, sort_order) VALUES
    ('L1', 'Left1',  1000323, '1-8.1', 1, 1),
    ('L2', 'Left2',  1002223, '1-8.2', 1, 2),
    ('L3', 'Left3',  1003323, '1-8.3', 1, 3),
    ('L4', 'Left4',  1005223, '1-8.4', 1, 4),
    ('L5', 'Left5',  1006323, NULL,    1, 5),
    ('R1', 'Right1', 1000324, '1-3.1', 1, 6),
    ('R2', 'Right2', 1002224, '1-3.2', 1, 7),
    ('R3', 'Right3', 1003324, '1-3.3', 1, 8),
    ('R4', 'Right4', 1005224, '1-3.4', 1, 9),
    ('R5', 'Right5', 1006324, NULL,    1, 10),
    ('LS', '直通左', 1007323, NULL,    1, 11),
    ('RS', '異常口', 1007301, NULL,    1, 12),
    ('NG', 'NG',     1007301, NULL,    1, 13);
