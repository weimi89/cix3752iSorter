# cix3752iSorter 重寫計畫

## Context

廠商交付的分揀機控制系統 `main_proj`（Go `ecs1000` + Node `twfilter/server.js`）有結構性問題：斷線時空指標當機、重連後控制器重複、鎖未釋放死鎖、在途件不持久化、取消／異常件詳情被啟動清理誤刪（已驗證 472 筆）、格口查詢同步阻塞皮帶控制器。且資料夾內 Go 原始碼不是正式機二進位的來源。使用者決定以 **Rust 單一執行檔**重寫，合併 Go 與 Node 兩支，網頁後台打掉重做，樣板對齊自家中介機 `cix3752iLabelPrint`（Tauri v2 + axum + Vue 3/Vuetify）。

- 新專案：`cix3752iSorter/`（已 `git init`，含 `README.md`、`docs/legacy-analysis.md`、`docs/protocol-spec.md`）
- 目標主機：工控機 **Ubuntu 20.04 → 日後升 22.04 / 24.04**，x86_64
- 中介機：`cix3752iLabelPrint`（`192.168.0.37:18080`），契約 `cix3752iLabelPrint/docs/local-http-api.md`、`device-alert-api.md`
- 規格依據：`docs/protocol-spec.md`（由 71 萬行 `cmd.log` 逆推）；廠商正式機原始碼到手後補「待確認清單」

## 關鍵設計決策

| 決策 | 選擇 | 理由 |
|---|---|---|
| 程式形態 | **Tauri 2 桌面程式**（2026-09-14 改，原本是純 headless 執行檔）：開視窗、登入後自動啟動；`--headless` 只給開發機接模擬器用（2026-09-15 拆掉服務版部署：現場要在視窗上啟停皮帶，沒畫面的服務版用不到） | 對齊 LabelPrint 的形態，現場工控機有畫面時直接當桌面程式用，沒畫面照舊當服務 |
| 相容目標 | 工控機 Ubuntu **20.04** x86_64（22.04／24.04 矩陣留註解，升級再開） | 20.04 沒有 webkit2gtk-4.1，要自編 glib 2.78 + libsoup3 + webkit 整套（沿用 LabelPrint `build-focal-stack` action）；連的棧不同，產物不能跨 distro 互換 |
| 建置方式 | 只走 GitHub Actions `release.yml`（三個 distro 各在自己的 container 建） | Tauri 要連目標平台的 gtk／webkit 開發套件，macOS 無法交叉編譯 Linux 版；本機只能建 macOS 版做開發驗證 |
| 自動更新 | Tauri updater 讀 `latest.json`，`platforms` 鍵帶 distro（`linux-x86_64-ubuntu-20.04`）裝 .deb | 程式照 `/etc/os-release` 選自己那筆；.deb 安裝需 root，桌面更新會跳系統密碼（pkexec），headless 換檔不需要 |
| 行程模型 | 每個裝置一個 tokio task 擁有自己的連線；`tracker` 單一 task 擁有全部包裹狀態；task 之間只用 channel | 消滅舊版共享鎖／nil 解參考／重複控制器三類病 |
| 相機接法 | 本程式直接開 `:8051` 收相機原始資料（挑碼邏輯移植 `checkcode1`），**不再有 8050／3000 中轉** | 相機設定不用改；少一次 HTTP 來回 |
| 格口查詢 | 條碼一到就非同步問中介機；`~O` 到時「有答案用答案、逾時走預設口」，皮帶迴圈永不阻塞 | `~P→~O` 實測預算 ≈1.1s，中介機 p99 898ms |
| 持久化 | 包裹 `~P` 時即寫入，之後只更新；每個訊號寫 `parcel_events`；列印任務落 `print_jobs`；SQLite WAL | 重啟不丟在途件與列印任務；詳情用事件表而非 JSON blob |
| 前後端通道 | axum REST `/api/*` + SSE `/events/stream` + rust-embed 內嵌 `dist`；桌面視窗只多一個 `backend_base_url` command 問到本機伺服器位址，其餘一律走 REST／SSE | 同一份前端在瀏覽器與 Tauri 視窗都能跑，不需要 `/rpc/{cmd}` 分派層；SSE 用戶端沿用 LabelPrint `src/api/events.js` |
| 設定 | `config.toml`（裝置位址、指令字串、NG 規則、燈號、印表機 USB 埠、列印 profile）+ DB `chutes` 表（代號 ↔ CID ↔ 印表機） | 對應舊 `conf.json` + `gkconfig.json` + `chute` 表；設定頁改完即熱套用，連線類參數改動觸發該裝置 task 重連 |
| 存取控制 | 內網免登入、**不設操作密碼**（2026-09-15 決定：程式已由自己維護，舊 `setPwd` 不再需要）；不做外網開放 | 現場 LAN 工具；LabelPrint 的 `server/auth.rs` 外網模型這裡用不到 |

## 專案結構

```
cix3752iSorter/
├── Cargo.toml                    單一 crate `cix3752i-sorter`（bin: sorter；bin: replay 模擬器）
├── build.rs                      dist 佔位（照 LabelPrint build.rs，去掉 tauri_build）
├── migrations/                   sqlx 編譯期 migration
├── src/
│   ├── main.rs                   bootstrap：log → config → db → devices → tracker → label → server → ctrl-c
│   ├── config/                   AppConfig（TOML，serde，Default）、載入／儲存（原子寫入，移植 fs_atomic.rs）、熱套用 watch channel
│   ├── db/                       SqlitePool 初始化（照 LabelPrint db/mod.rs，路徑改 CLI/環境變數）
│   ├── event_bus.rs              broadcast 匯流排（照 event_bridge.rs，去掉 Tauri）
│   ├── event_log.rs              照 LabelPrint（fire-and-forget 寫 event_log）
│   ├── protocol/                 純函式：belt.rs / sorter.rs 訊號解析、command.rs（Kn/Kx/Ka/KL/KM）、cid.rs（CID 編解碼）
│   ├── device/
│   │   ├── line_client.rs        通用文字行 TCP client：DialTimeout、TCP keepalive、read deadline、退避重連、CancellationToken；輸入 mpsc<Command>、輸出 mpsc<Line>
│   │   ├── belt.rs / sorter.rs / sysled.rs   把 Line 解析成 DeviceEvent 送 tracker；把 tracker 的 DeviceCommand 送線上
│   │   └── camera.rs             TCP server :8051、幀解析、挑碼（移植 checkcode1）、5 秒去重 → BarcodeEvent
│   ├── tracker/                  包裹狀態機（單 task）：slot/cart 對應、綁碼窗口、~O 決策、Kx 判斷、堵塞／丟失、NG 停線、燈號、計數；每個轉移寫 DB 並 emit 事件
│   ├── chute/                    格口解析：barcode → 中介機 → channel_code → chutes 表 → CID；oneshot 回 tracker；逾時／NoRead／LS／RS 規則
│   ├── middleware/               reqwest client：GET /api/parcel、POST /api/report（佇列＋指數退避，移植 queue/mod.rs 的 claim/backoff SQL）、POST /api/device-alert（卡件、印表機、USB 斷線，含節流）
│   ├── label/
│   │   ├── raster.rs             image：依 profile 縮放（contain、留白）、灰階、門檻 200、1-bit 打包
│   │   ├── tspl.rs               512 零前導 + SIZE/cmd/CLS/BITMAP/PRINT
│   │   ├── usb.rs                /sys/class/usbmisc 對 bus-port → /dev/usb/lpN
│   │   └── queue.rs              print_jobs 表；每台印表機一個 worker（spawn_blocking 寫裝置）、重試／告警／延遲 (格口號-1)*400ms
│   ├── server/                   axum：routes.rs（REST）、events.rs（SSE，照 LabelPrint）、assets.rs（照 LabelPrint）
│   └── sim/                      replay 模擬器：當皮帶+分揀機 TCP server 回放 cmd.log、假相機、假中介機
├── frontend/                     Vue 3 + Vuetify + Pinia + vue-router（從 LabelPrint 的 @core/@layouts/plugins 起樣板）
└── docs/                         legacy-analysis / protocol-spec / plan / handover
```

## 資料表（migrations/0001_init.sql）

| 表 | 用途 | 重點欄位 |
|---|---|---|
| `parcels` | 每件包裹主檔，`~P` 即建立 | `id`（自增）、`ulid`、`barcode`、`chute_code`、`chute_cid`、`status`（1–8 沿用舊語意）、`belt_slot`、`cart`、`started_at`、`ended_at`、`travel_ms`、`response_id`、`chute_source`（api/default/noread/timeout） |
| `parcel_events` | 訊號時間軸（取代舊 `detail` JSON） | `parcel_id`、`ts_ms`、`source`（belt/sorter/camera/api/tracker）、`kind`（P/L/O/E/Kn/c/j/g/e/k/u/Kx…）、`raw` |
| `print_jobs` | 列印佇列 | `parcel_id`、`chute_code`、`printer_port`、`profile`、`tspl_path`（點陣落地檔）、`status`、`attempts`、`last_error`、`not_before` |
| `report_queue` | 中介機回報佇列 | 移植 LabelPrint 欄位：`response_id`、`status`、`retry_count`、`next_retry_at` |
| `chutes` | 格口對照 | `code`（L1…R5/LS/RS/NG）、`cid`、`printer_port`、`enabled`、`sort_order` |
| `event_log` | 系統事件 | 照 LabelPrint |
| `daily_stats` | 每日件數快取 | 避免每次啟動 `count(*)` |

保留政策：`retention_days` 只依 `started_at` 清 `parcels`（級聯清 `parcel_events` / `print_jobs`），**不再有以 0 為門檻的欄位**。

## REST / SSE 介面（前端唯一資料來源）

- `GET /api/status`：裝置連線狀態、皮帶運行、今日件數、在途件、當前件、最近系統訊息
- `GET /api/parcels?…`、`GET /api/parcels/{id}/events`、`GET /api/parcels/export.xlsx`
- `GET/PUT /api/config`、`GET/PUT /api/chutes`
- `POST /api/belt/{start|auto|stop}`、`POST /api/led`、`POST /api/sorter/reset`
- `GET /api/print-jobs`、`POST /api/print-jobs/{id}/retry`、`POST /api/printers/{port}/test`
- `GET /api/ir/status`（`Kd[`）、`POST /api/ir/{block|unblock}`（M6）
- `GET /api/logs?…`（event_log）
- `GET /events/stream`：`status`、`parcel-updated`、`device-state`、`system-message`、`print-job`

## 網頁頁面（frontend/src/pages）

Dashboard（即時：當前件、在途件表、件數、裝置燈號、系統訊息、皮帶啟停鈕）、Parcels（歷史查詢 + 詳情時間軸 + 匯出）、PrintJobs、Chutes、DeviceSettings（皮帶／分揀機／燈號／NG 規則／相機）、PrinterSettings（USB 埠、profile、測試列印）、EventLog、IrCheck（M6）。繁中為主，掛 vue-i18n 以便日後加越文。

## 里程碑

| # | 內容 | 完成判準 |
|---|---|---|
| **M0 骨架與建置** | Cargo 專案、config/db/event_bus/event_log/server 骨架、內嵌 dist 佔位、`cargo-zigbuild` 產 glibc-2.31 二進位、GHA release 工作流、systemd/supervisor 範本、部署腳本 | 二進位在工控機 20.04 上跑起來、能開網頁殼、`cargo test` 綠 |
| **M1 裝置層 + 模擬器** | `protocol/` 全部訊號解析（單元測試覆蓋 spec 每一種）、`line_client` 斷線／重連／keepalive、`sim/replay` 回放 `cmd.log`（保留原時序，可加速）、假相機 | 模擬器跑完 40,870 件，本程式不當機、無 goroutine 等價洩漏（task 數穩定）、每種訊號都有解析 |
| **M2 包裹狀態機** | `tracker`：slot/cart 對應、綁碼、`~O` 決策（暫用預設口）、Kx 規則、堵塞／丟失、NG 停線、燈號、計數；`parcels` / `parcel_events` 落 DB | 回放後每件狀態與舊 `sortAi.db` 對應期間的 `status` 分佈一致（3/4/5/6/7/8 比例差異 <1%），逐件抽查 50 筆時間軸相符 |
| **M3 格口與中介機** | `chute/`、`middleware/`（parcel、report 佇列、device-alert）、`chutes` 表、假中介機（可注入延遲／NoRead／LS） | 注入 p99 900ms 延遲時「走預設口」比例 <0.1%；1.2s 以上才走預設口；report 全數送達或進退避佇列 |
| **M4 列印** | `label/` 四個模組、`print_jobs` worker、USB 斷線／寫入失敗告警與節流 | 用 `twfilter/app/pic/test*.png` 產出的 TSPL 與舊 Node 逐位元組一致（同一輸入）；拔線→重插情境任務不丟 |
| **M5 網頁後台** | REST + SSE + 全部頁面；設定熱套用；xlsx 匯出 | 每頁實機操作驗證；設定改動不重啟即生效（連線參數觸發重連） |
| **M6 現場切換** | 設定轉換腳本（`conf.json` + `gkconfig.json` + `chute` 表 → `config.toml` + `chutes`）、IR 檢查頁、supervisor 切換與回退步驟、交接文件 | 現場用新版跑一個班次；回退只需切 supervisor 程式路徑 |

M1–M4 在**沒有實機**的情況下全部可用模擬器驗證；每個里程碑收尾寫 `docs/handover.md`。

## 可直接移植的 LabelPrint 程式

- `src-tauri/src/server/assets.rs`（去 `__CIX_WEB__` 旗標或保留）、`server/events.rs`（去 auth recheck）、`event_bridge.rs`（去 Tauri emit）
- `db/mod.rs`（路徑改由 CLI）、`event_log.rs`、`fs_atomic.rs`、`log/mod.rs`
- `queue/mod.rs` 的 `SQL_CLAIM_FOR_SENDING` / `backoff_seconds` 模式
- 前端：`src/@core`、`src/@layouts`、`src/plugins/vuetify*`、`src/api/events.js`（SSE 單連線分發）、`src/layouts/DefaultLayout.vue`、`EventLogPage.vue` / `QueueLogPage.vue` 版型
- `.github/workflows/release.yml` 的容器建置骨架、`tests/docker-ubuntu-build.sh` 思路

## 驗證方式

1. **單元**：`protocol/`（每種訊號正反例、CID 編解碼往返、`Kn` 產生對照 `cmd.log` 實例）、`label/raster+tspl`（固定輸入的位元組快照）、`chute` 規則、退避函式。
2. **整合（模擬器）**：`cargo run --bin sorter-replay -- --log main_proj/data/cmd.log --speed 20` 起皮帶／分揀機／相機／中介機假伺服器，主程式對接；斷言 DB 內狀態分佈、逾時比例、無 panic、記憶體與 task 數平穩；反覆拔線（模擬器隨機關 socket）驗重連。
3. **對照舊資料**：同時段舊 `sortAi.db` 的 `status` 分佈與 `chute` 分佈當基準。
4. **建置**：`cargo-zigbuild` 產物用 `objdump -T | grep GLIBC_` 確認最高版本 ≤ 2.31；GHA 產物在工控機 20.04 實跑。
5. **實機**（M5/M6）：每個頁面、每個裝置控制鈕、拔印表機 USB、關中介機、關分揀機電源各做一次，對照事件 log 與告警。
