-- 網頁後台對外存取的登入狀態。
--
-- 內網來源免登入（現場電腦、工控機、手機），外網來源必須輸入共用密碼。
-- 密碼本身（argon2 雜湊）存在 app_setting，不放設定檔 ——
-- 設定檔會被 GET /api/config 整包回給前端，密碼雜湊不該跟著跑到瀏覽器。

CREATE TABLE IF NOT EXISTS app_setting (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- 已登入的連線。token 存 SHA-256 而非原文：資料庫外流時光有雜湊冒用不了。
CREATE TABLE IF NOT EXISTS web_session (
    token_hash   TEXT PRIMARY KEY,
    client_ip    TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    expires_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_web_session_expires ON web_session(expires_at);

-- 登入失敗次數，用來鎖住暴力嘗試。
-- 對外只有一組共用密碼，沒有這層的話等於讓人慢慢猜到天亮。
CREATE TABLE IF NOT EXISTS web_login_attempt (
    client_ip    TEXT PRIMARY KEY,
    fail_count   INTEGER NOT NULL DEFAULT 0,
    last_fail_at TEXT,
    locked_until TEXT
);
