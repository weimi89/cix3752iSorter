-- 異常口處理清單：落到異常口（預設格口）的件，現場要有人下架或重投才算結案。
-- 清單本身從 parcels 撈（今天、落到預設格口、狀態完成），這張表只記「處理了沒」：
-- removed = 已下架（關轉、查無訂單這類要退回的）、refed = 已重投（讀碼失敗、逾時這類再走一趟就好）。
-- handled_by：web / desktop / auto（同條碼再進線且中介機正常給格口 → 自動視為已重投）。
CREATE TABLE abnormal_handling (
    parcel_id  INTEGER PRIMARY KEY REFERENCES parcels(id) ON DELETE CASCADE,
    state      TEXT    NOT NULL,
    handled_ms INTEGER NOT NULL,
    handled_by TEXT    NOT NULL,
    note       TEXT
);
