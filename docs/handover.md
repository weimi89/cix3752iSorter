# 交接紀錄

> 依 `docs/plan.md` 里程碑推進；狀態只填實際驗證過的結果。

## 2026-09-14

### M0 骨架與建置 — 完成（本機驗證）

| 項目 | 狀態 | 驗證方式 |
|---|---|---|
| Cargo 專案、`config`/`db`/`event_bus`/`event_log`/`fs_atomic`/`log`/`server` 骨架 | ✅ | `cargo check`、`cargo test`（7 個測試綠） |
| `config.toml` 不存在時自建預設檔；缺區段以預設補齊 | ✅ | 冒煙測試看到 WARN「已用預設值建立」，檔案內容正確 |
| SQLite WAL + `migrations/0001_init.sql`（7 張表 + 格口初值） | ✅ | 冒煙後 `.tables` 列出全部表 |
| `GET /api/health`、`/api/status`、`/api/logs`、`/events/stream`、SPA fallback | ✅ | curl 逐一打過；`/events/stream` 目前尚無事件來源，只驗到連線不報錯 |
| 交叉編譯 glibc 2.31（`scripts/build-linux.sh`） | ✅ | 產物 x86_64 ELF，最高需求 `GLIBC_2.30`，只依賴 libc/libm/libdl/libpthread |
| GHA `release.yml`（ubuntu:20.04 容器） | ⏳ 未跑過 | 尚未推到 GitHub；推上去打 tag 後要看一次 |
| `deploy/`：systemd 單元、supervisor 設定、`install.sh` | ⏳ 未實機 | 需在工控機執行 `sudo bash install.sh systemd` 驗證 |
| **在工控機 20.04 實跑** | ⏳ **待主人操作** | 把 `dist-bin/cix3752i-sorter_0.1.0_linux-x86_64.tar.gz` 丟到工控機解開、`./sorter --config config.toml --data-dir data`，瀏覽器開 `http://<ip>:18090/api/health`（預設埠已避開舊系統的 8080） |

### 已知待辦

- 前端 `frontend/` 尚未建立（M5），目前內嵌的是 `build.rs` 的佔位頁。
- `deploy/` 的服務範本以 `__APP_USER__`／`__APP_DIR__` 佔位，`install.sh` 依執行 `sudo` 的帳號代入。
- 廠商正式機原始碼尚未到手：`docs/protocol-spec.md` 第 10 節待確認清單。

### M1 裝置層 + 模擬器 — 完成

| 項目 | 狀態 | 驗證方式 |
|---|---|---|
| `protocol/`（belt/sorter 解析、Kn/Kx/Ka/KL 組裝、CID） | ✅ | 15 個單元測試，含 `cmd.log` 實例對照 |
| `device/line_client.rs`（keepalive、讀逾時、退避重連、取消、斷線丟棄待送指令） | ✅ | 3 個測試（假裝置：收行／送指令／掛斷重連／讀逾時） |
| `device/camera.rs`（:8051 收幀、挑碼規則移植、5 秒去重） | ✅ | 7 個測試 |
| `bin/replay.rs`（回放 cmd.log 當皮帶／分揀機／相機；`--drop-every` 隨機掛斷；`--gate-c` 維持 Kn→~c 因果） | ✅ | 回放 09-07 19:00–19:20（7,741 行）逐種訊號計數與原始日誌**完全吻合**；掛斷後自動重連 |

**踩過的坑**：`select!` 裡的讀取必須用 cancel-safe 的 `Lines::next_line`；用 `read_line` 會在送指令那一刻弄丟剛到的行。

### M2 包裹狀態機 — 完成（模擬器驗證）

| 項目 | 狀態 | 驗證方式 |
|---|---|---|
| `tracker/machine.rs`（綁碼窗口、頭部排隊、Kn/Kx、堵塞／丟失／取消、停線規則、燈號、兜底逾時） | ✅ | 16 個單元測試（`machine_tests.rs`，含每條停線規則） |
| `tracker/store.rs`（~P 即落 DB、事件表、daily_stats、啟動時孤兒件收尾） | ✅ | 回放後 `parcels`/`parcel_events` 內容檢查 |
| 與舊系統對照 | ✅ | 09-07 19:00–19:20 @20x：新 598/4/19/8（狀態 3/4/6/7）vs 舊 600/4/19/8；整個 09-07 班次 10,205 件 @50x：9871/78/133/105/15 vs 9887/77/134/86/15，RSS 20MB、結束時在途 3 件 |

**未做／待確認**：多集群（`Ka`/`~n`）未實作（現場單集群）；`~I` 建件未實作（現場關閉）；50x 回放的 19 件額外取消是回放因果的殘餘，20x 完全吻合。

### M3 格口與中介機 — 完成（假中介機驗證）

| 項目 | 狀態 | 驗證方式 |
|---|---|---|
| `middleware/mod.rs`（`/api/parcel`、面單下載、`/api/report`、`/api/device-alert`，逾時與錯誤分類） | ✅ | 回應解析單元測試（正常／NOREAD／錯誤面單） |
| `middleware/report_queue.rs`（落表、指數退避、4xx 永久失敗、重啟續送） | ✅ | 退避函式測試；1x 回放 144 筆全部 success |
| `chute/mod.rs`（`decide()` 純函式：NOREAD／業務錯誤／停用格口／LS 直通；每請求獨立 task） | ✅ | 6 個單元測試 |
| 模擬器假中介機（`--mw-delay-ms`、`--mw-error-every`，`/images` 回 PNG） | ✅ | 1x 回放 4 分鐘：api 144／default 3／noread 5／timeout 0（延遲中位 386ms、max 563ms）；延遲拉到 1s 中位：timeout 31、遲到回覆 31 筆只記錄不改格口 |

**決策點（要主人拍板）**：回報時機沿用舊系統——**拿到格口就回報**（`chute_decided` 入列），不是等 `~e` 掉落；遲到回覆（已走預設口）**不回報**。若要改成「實際落格口才回報」，改 `tracker/mod.rs` 的 `chute_decided` → 在 `end_parcel(Done)` 時入列即可。

### M4 列印 — 完成（假印表機驗證）

| 項目 | 狀態 | 驗證方式 |
|---|---|---|
| `label/raster.rs`（profile 尺寸、contain 縮放、2mm 留白、門檻 200、1-bit 打包） | ✅ | 6 個單元測試；`CIX_PREVIEW_OUT=… cargo test 匯出點陣預覽 -- --ignored` 匯出預覽圖肉眼確認 |
| `label/tspl.rs`（512 零前導 + SIZE/cmd/CLS/BITMAP/PRINT；測試頁） | ✅ | 位元組佈局測試 |
| `label/usb.rs`（`/sys/class/usbmisc` 埠位對應；列出印表機） | ✅ | 假 sysfs 符號連結測試 |
| `label/queue.rs`（`print_jobs` 落地、每埠位一個 worker、`spawn_blocking` 寫入 + 15s 逾時、重試／告警節流 60s、一小時放棄、重啟續印） | ✅ | 1x 回放 90 秒：43 件入列、36 件印出；故意不接 L4 → 7 件 pending + `USB_DISCONNECT` 告警送達假中介機（每 60s 重提醒）→ 接上後 8 秒內全部印出、spool 清空 |
| 開發機沒有 USB 印表機 | — | 設 `CIX_PRINT_FAKE_DIR=<dir>`，目錄下存在名為埠位的檔案即當作印表機（測試就是這樣跑的） |

**與舊版的差異**：縮放用 Lanczos3（Node 用 sharp 預設），像素不會逐位元組相同，但尺寸、留白、二值化門檻、TSPL 佈局一致；正式機上請先印一張對比。

### M5 網頁後台 — 完成（本機瀏覽器實測）

| 項目 | 狀態 | 驗證方式 |
|---|---|---|
| REST：status／parcels（查詢、詳情、xlsx 匯出）／stats／config／chutes／belt／sorter／print-jobs／printers／report-queue／logs | ✅ | curl 逐一打過（含密碼錯誤 403、CID 不合法 400、指令白名單 400） |
| 前端 `frontend/`（Vue 3 + Vuetify，樣板移植自 cix3752iLabelPrint；離線圖示子集） | ✅ | Chrome 逐頁開過：即時看板（裝置燈、皮帶啟停、件數、當前件、在途表、14 天圖、系統訊息）、包裹查詢 + 詳情時間軸、列印任務、回報佇列、事件記錄、格口對照、印表機、系統設定六個分頁 |
| 設定密碼流程（錯誤提示／正確存檔／sessionStorage 記住） | ✅ | 實際操作 |
| SSE 即時更新（狀態 0.5s、包裹、列印、回報、系統訊息） | ✅ | 看板數字隨回放跳動 |
| 主題 | ✅ 與 cix3752iLabelPrint 同一套（乖乖綠主色、半暗側欄，`useThemeApply` + `stores/theme.js`） | 實機比對 |
| 語系 | 主人決定**不做多語系**：只留繁中，切換選單與越南文檔已移除（vue-i18n 保留當字串表用） | — |
| 版型 | 全部頁面對齊 cix3752iLabelPrint：`AppHeader` 頁首（圖示／標題／副標／動作列，手機收進選單）、`card-shadow` 統計卡、「進階查詢」展開面板、`TablePagination` 表頭表尾雙分頁（每頁 25/50/100…）、`table-cards` 手機卡片式表格、格口對照左右欄卡片 | Chrome 逐頁比對 |
| 記錄頁 | 列印任務／回報佇列／事件記錄都有關鍵字搜尋＋狀態篩選＋分頁（後端 total/list）；表格不顯示內部流水號 | 實機操作 |
| 自動更新 | ✅ 對齊 LabelPrint：後端定期讀 GitHub Release 的 `latest.json`（`src/updater/`），導覽列出現下載徽章 → 對話框顯示版本說明 → 密碼 → 串流下載＋SHA-256 校驗 → 新檔先 `--version` 自檢 → `rename` 蓋掉執行檔 → 1 秒後結束交給 supervisor／systemd 重啟 → 前端輪詢 health 自動重載；沒外網可在同一對話框上傳 tar.gz | 本機以假發版（0.1.0 → 0.1.1、假 supervisor 迴圈）走完整流程，畫面自動變 v0.1.1 |
| 系統設定頁 UX | 段落捷徑列（貼頂）、每段卡片圖示標題＋副標、裝置段顯示連線狀態並可「測試連線」（`POST /api/devices/test`，只做握手不送指令）、位址／網址格式即時檢查（有錯不送出）、有改動才能儲存＋底部未儲存列（放棄／儲存）＋離頁提醒、停線規則改成清單列、列印紙張改成卡片、密碼可顯示 | Chrome 實測：測試連線成功／拒絕／逾時三種、格式錯誤警示、放棄變更還原、存檔落到 config.toml、離頁攔截 |
| 未做 | IR 光電檢查頁（M6）；手機遙控頁 | — |

**發版方式**（2026-09-14 改為三 distro Tauri 發版，見下一節）：`src-tauri/Cargo.toml` 與 `src-tauri/tauri.conf.json` 版本號一致 → `git tag -a vX.Y.Z -m "版本說明"` → push tag → GHA `release.yml` 在 ubuntu:20.04／22.04／24.04 三個 container 各建 .deb 與 headless tar.gz、合併 `latest.json` 上傳到 **draft** Release → 到 Releases 頁公開；工控機最多一小時內看到新版（`[update] check_interval_min`）。tag 版本與兩個設定檔不一致會被 GHA 擋下。GitHub repo 預設 `weimi89/cix3752iSorter`，建 repo 後若名稱不同要改 `config.toml` 的 `update.endpoint` 與 `src-tauri/tauri.conf.json` 的 `plugins.updater.endpoints`。
**踩過的坑**：設定密碼對話框必須是全站單例（掛在 DefaultLayout），composable 裡 `ensure()` 才等得到；各頁各掛一個會永遠等不到。
**開發方式**：`yarn dev`（Vite :5180 代理到後端 :18090，`CIX_BACKEND` 可改）或 `yarn tauri dev` 直接開視窗；正式建置 `yarn build` 後 `cd src-tauri && cargo build` 內嵌。
**踩過的坑**：Chrome 自動化的分頁若在背景（`visibilityState=hidden`），`requestAnimationFrame` 不跑，所有 Vuetify 過場（對話框、底部列）會停在 opacity 0，看起來像沒出現；驗過場效果前先確認分頁在前景，或改查 DOM。
**踩過的坑**：`vue-i18n` 的 `useI18n` 不在 AutoImport 清單，頁面要自己 import；空字串查詢參數前端不送、後端也當不篩選（`non_empty`）。

### Tauri 桌面化 + Linux 三 distro 發版 — 程式碼完成，GHA 未跑過

| 項目 | 狀態 | 驗證方式 |
|---|---|---|
| 專案改成 Tauri 2 桌面程式（`src-tauri/`），預設開視窗、`--headless`／`CIX_HEADLESS=1` 只跑服務；兩種模式共用 `app::bootstrap` | ✅ | macOS：headless 模式接模擬器跑完整流程；桌面模式開視窗、前端載入、看板數字跟著回放跳動 |
| 前端雙執行環境（`src/api/runtime.js`）：Tauri 內向 Rust 問 `backend_base_url`，瀏覽器用相對路徑 | ✅ | 同上 |
| 自動更新雙路徑（`src/composables/useUpdater.js`）：桌面走 Tauri updater、瀏覽器走後端 `/api/update/*` | ✅ 瀏覽器路徑本機驗過；Tauri 路徑要等真的有 Release 才驗得到 | — |
| `latest.json` 平台鍵帶 distro（`updater::platform_tag`：`linux-x86_64-ubuntu-20.04`、`…-headless`）；桌面 updater 在 Linux 設同樣的 target | ✅ 單元測試（os-release 解析、混合 manifest 解析） | `cargo test` 71 綠 |
| 模擬器執行檔改名 `sorter-replay`（Tauri 會把所有 bin 一起打進 .deb 的 /usr/bin，避免撞名） | ✅ | `cargo metadata` 目標清單 |
| `release.yml`：setup（distro 清單單一來源）→ create-release（版本一致性檢查、draft 幂等）→ release-linux（矩陣三 distro，20.04 走 `build-focal-stack` 自編棧）→ publish-manifest（三份齊才合併上傳 `latest.json`、驗 asset 到齊） | ✅ 2026-09-15 v0.1.0 三 distro 全過 | 見下方「v0.1.0 發版」 |
| `deploy/install.sh` 改為安裝包腳本：`desktop`（裝 .deb）／`systemd`／`supervisor`（從 .deb 取執行檔放 APP_DIR，服務帶 `--headless`）；20.04 先鋪 `stack/` 到 /usr/local | ⏳ 未實機 | `bash -n` 過；要在工控機跑 |
| `scripts/build-linux.sh`（zig 交叉編譯）已搬到 `backups/`：Tauri 版無法從 macOS 交叉編譯 Linux | — | — |

**要主人做的事（GitHub UI）**：建 repo `weimi89/cix3752iSorter`（或改兩處 endpoint）→ Settings → Secrets 加 `TAURI_SIGNING_PRIVATE_KEY`（`~/.tauri/cix3752iSorter.key` 內容）與 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（建金鑰時設的密碼，沒設就留空字串）→ 手動跑一次 `warm-focal-cache.yml`（在 main 上）把 20.04 的 webkit 棧先編好進快取 → 打 `v0.1.0` tag 觸發 `release.yml`。
**已知風險**：Tauri updater 在 Linux 裝 .deb 走 `pkexec dpkg -i`，桌面模式按「立即更新」會跳系統密碼；現場若不想每次輸入，改用 headless 模式（換檔不需 root）。`dist-bin/` 內的舊 tar.gz 是 Tauri 化之前建的，不要再拿去工控機。

## 2026-09-15

### 舊系統移植缺口稽核 — 已修 7 項，2 項未做

逐條對照 `main_proj/logic/*.go` 與 `twfilter/app/server.js` 後找到的缺口（主流程本身一致，漏的是流程邊上的規則）：

| # | 缺口 | 修法 | 驗證 |
|---|---|---|---|
| 1 | 皮帶連線後沒有先停止＋重置（舊版「避免傷人」） | `device/belt.rs` 連上依序送 `belt.cmd.stop`、`belt.cmd.reset`；初始化指令改成 watch，設定改了下次連線生效 | 模擬器：連上即收到 `KM998 1`、`KM999` |
| 2 | 格口回覆太晚仍印面單（包裹已走預設口） | 面單改由**狀態機接受決定後**才送列印（`Outputs::print_label`），解析器不再自己丟列印工作 | 單元測試 `格口被接受才印面單_回覆太晚不印` |
| 3 | 面單抓不到，包裹仍分到正常格口並回報 | 解析器**先抓面單**才回結果（對齊舊 Node）；抓不到 → 走預設口、不回報、不印（`chute/mod.rs resolve`） | 模擬器 `--mw-img-fail-every 4`：失敗件走 RS、`report_queue` 無該件 |
| 4 | NoRead 沒呼叫中介機（契約要它拍照存證＋計數） | NoRead 本機立刻決定預設口，同時送 `notify_only` 請求打 `/api/parcel/NoRead`，回覆不改決定 | 假中介機印出 `NoRead 通知 #n` |
| 5 | 裝置原始訊號沒留檔（舊 `cmd.log` 是逆推協定的唯一依據） | `device/signal_log.rs`：`data/logs/signals-YYYY-MM-DD.log`，收送每行、連線狀態、相機幀與挑出的碼；`~k-1` 每裝置 60 秒一次 | 模擬器跑完看檔 |
| 6 | 桌面模式完全沒有日誌（stdout 被丟掉） | `log.rs` 加 `attach_file`：`data/logs/sorter.YYYY-MM-DD.log` 逐日輪替（tracing-appender），headless 與桌面都有 | headless 啟動看檔 |
| 7 | `retention_days` 有設定沒實作 | `db/retention.rs`：啟動＋每小時清 `parcels`（級聯 events）、`print_jobs`（連點陣檔）、已完成的 `report_queue`、`event_log`；以**上線時間**為準，不重蹈舊版拿結束時間 0 誤刪的覆轍 | 單元測試 `只清超過天數的資料_未送出的回報保留` |
| 8 | 急停按鈕互鎖（舊版「急停恢復」要等按鍵放開） | **未做**：現場 `emergencyButton: []` 沒接 | — |
| 9 | 快速分揀 API `/c4/qs`、`/live/control/detail` | **未做**：要先確認現場有沒有外部系統在打 | — |

**副作用要知道**：#3 之後「有面單的件」要 API＋面單都在 `~O` 前到齊才走正確格口，否則走預設口（舊 Node 同樣是 1200ms 總預算）；`middleware.parcel_timeout_ms` 與 `label_timeout_ms` 各自獨立，真正的截止線仍是 `~O`。**目標格口沒接印表機（L5／R5／LS）的件不抓面單**，圖片服務故障不會把它們拖去異常口（這點比舊 Node 寬鬆，舊版一律 RS）。
**兩層覆檢後順手修的**：`retention` 只清已結束（done／failed）的列印任務，還在等印表機的不動；狀態機佇列滿時格口結果丟棄會記 error；面單圖超過 8MB 拒收；網頁啟停皮帶會立刻更新運轉狀態（皮帶運轉中沒有心跳，單顆啟停鈕靠這個切換，`~k-1` 會糾正）；headless 也處理 SIGTERM（supervisor／systemd 停服務用的），結束前 `log::flush()` 把日誌尾段落檔。
**已知但沒改**：`retention_days` 改了之後，`signals-*.log`／`sorter.*.log` 的保留天數要**重啟才生效**（DB 清理是每小時讀最新設定）；`alarm_on_start` 紅燈在燈所在那條線**每次連上**都亮（含斷線重連），與舊系統一致。
**開發機注意**：舊 session 留下的測試行程（`scratchpad/run/supervise.sh` 會不斷拉起舊版 `sorter` 占 8051／18090）2026-09-15 已清掉；再看到 18090 被占，先找 `supervise.sh`。

### 穩定性（B）與體驗（C）加強 — 完成（模擬器＋瀏覽器實測）

| 項目 | 做法 | 驗證 |
|---|---|---|
| 綁碼誤配連鎖 | `~O` 後該件不再參與綁碼（對齊舊版）；窗口內多件候選挑「`~P`→條碼」最接近 `camera.bind_expected_ms`（預設 226）的 | 單元測試：前一件漏拍時第二件的條碼不會被搶走；`~O` 後才到的條碼不綁 |
| 卡件告警重置 | `JamThrottle`：同格口 20 秒一次，但安靜 5 秒（`middleware.jam_alert_reset_ms`）後下一個 `~k` 立刻再報；每個 `~k` 都交給它判斷 | 單元測試 |
| 今日件數口徑 | 維持「今日進入皮帶的包裹數（含異常件與在途件）」，看板卡片滑鼠停留有說明 | — |
| 日誌保留天數熱套用 | `signal_log` 每次輪替與每小時讀最新 `retention_days`，`sorter.*.log` 一起清（不再用 tracing-appender 的 max_log_files）；訊號密集時每 0.5 秒也 flush | 單元測試（兩種前綴一起清） |
| 急停互鎖 | 按鈕動作新增 `estop`（按住鎖住）／`estop_release`（急停放開才啟動）；設定頁下拉多兩項 | 單元測試 `急停互鎖_…`；設定頁選項實測 |
| 面單預覽 | `GET /api/print-jobs/{id}/preview.png`：從點陣檔還原；列印任務頁「預覽」鈕（待印／失敗才有，已印不保留） | 瀏覽器：預覽對話框；curl 800×1200 PNG；已印回「不保留點陣檔」 |
| 時間軸異常段 | 包裹詳情：`Kn→~c`>300ms、`~j→~g`>550ms、`~P→~O`>2s、`chute_late` 標紅並附說明 | 植入測試件四種都標到 |
| 格口查詢延遲 | 解析器保留最近 2000 筆耗時（含面單下載），`/api/status.chute_latency` 給最近一小時 p50/p90/p99、超預算比例（預算 1100ms）；看板新卡片，超預算轉紅 | 看板實測 |
| 日誌檔下載 | `GET /api/logs/files`、`/api/logs/files/{name}`（檔名白名單）；事件記錄頁「下載日誌檔」選單 | 選單列出程式日誌與裝置訊號兩檔 |
| 手機遙控 | `/control` 獨立 HTML（皮帶狀態＋單顆啟停、裝置燈、今日／在途、目前處理中；SSE 即時）；導覽列「手機遙控」對話框列區網網址＋QR（`/api/lan-ips`） | 遙控頁按鈕實際送出停止／啟動；QR 與網址顯示 |
| **沒做** | 快速分揀 API `/c4/qs`（要先確認有沒有外部系統在打）、`~I` 建件、多集群 | — |

**覆檢後補的**：皮帶斷線或急停按鈕設定改了就清掉急停鎖（斷線期間放開按鈕不會有 `~v`，不清會永遠鎖住）；存設定只換卡件節流參數、不重算正在節流的格口；區網 IP 開的網頁沒有剪貼簿 API，改提示長按複製；遙控頁遇到非 JSON 錯誤頁不再顯示解析錯誤。

### IR 光電檢查頁 — 完成（模擬器＋瀏覽器實測）

| 項目 | 做法 | 驗證 |
|---|---|---|
| 協定 | `protocol/ir.rs`：`Kd[` 回覆每台 8 位十六進位（全 0 = 正常）；`_1{9` → `U<m2>0 1;Y<m2>0 p1` → `_` 查單台每顆讀值（< 1000 = 被遮蔽）；`KY<m2>0 m -999／-1` 屏蔽／解除 | 單元測試；訊號日誌看到與舊 `ir.go` 一致的指令序列 |
| 等回覆 | `tracker::IrChannel`：網頁送指令後等狀態機 task 攔到 `~[…]`／`p1` 行（1.5 秒逾時），同時間只跑一個查詢 | curl 三個端點 |
| 端點 | `GET /api/ir/status`、`POST /api/ir/detail {m2}`、`POST /api/ir/block {m2, block}`（需設定密碼，寫事件記錄） | 模擬器對 `Kd[`／`p1` 造回覆 |
| 頁面 | 側欄「光電檢查」：每台一格（綠正常／紅閃有遮蔽／灰沒回應）→ 點開看 38／45／26 顆的環狀排列（沿用舊頁對照表），被遮蔽的閃紅（該台異常）或閃黃（該台正常）→ 屏蔽／解除屏蔽走設定密碼 | 瀏覽器：第 3 台紅、5 與 17 號光電亮、屏蔽送出 200 |

**待正式機確認**：`p1` 回覆的實際行格式（`protocol-spec` §10），目前解析比照舊程式（去 `_`、`=` 後取整數）；API 回應帶 `raw` 原文，現場可對照。

### 取消設定密碼 — 完成

主人決定：程式已由自己維護，不再需要舊系統的操作密碼。整套機制拆掉：後端 `require_password`／`/api/auth/check`／`server.settings_password` 移除（舊 `config.toml` 裡的欄位會被忽略），前端密碼對話框、composable、設定頁密碼欄、`X-Settings-Password` 標頭一併移除；設定、格口表、重置分揀機、光電屏蔽、更新安裝現在直接執行。

### 切換／回退文件 — 完成；supervisor 設定檔名修正

- `docs/cutover.md`：依正式機抓回來的 `main_proj/supervisor/`（程式名 `main_proj`＝Go、`twfilter`＝Node，皆 root；`sort_box` 已停用）寫成可直接貼的指令：前一天裝好不起 → 停舊（先 Node 放掉 8051，再 Go）→ 確認埠位釋放 → 改副檔名讓舊的不再自啟 → 起新 → 7 項驗證；回退反向、2 分鐘內舊程式恢復。
- **修了一個會讓 supervisor 模式裝不進去的錯**：現場 `supervisord.conf` 只 include `conf.d/*.ini`，`install.sh` 原本裝成 `.conf` 會被無視。已改成 `cix3752i-sorter.ini`（`deploy/supervisor-sorter.ini`）。
- 舊後台日誌（`main_proj.log*`）裡所有請求都來自 `127.0.0.1`、沒有 `/c4/qs`，**確認沒有外部系統在打快速分揀 API**，不移植。

### v0.1.0 發版 — GHA 三 distro 全部成功（draft，待公開）

- 2026-09-15 推上 `weimi89/cix3752iSorter`，`warm-focal-cache.yml` 首次自編 20.04 webkit 棧約 1 小時 55 分；`release.yml` 打 `v0.1.0` 跑了三次才過，兩個問題都在 20.04「帶走自編 .so」那段：① `ldconfig -p | awk '…exit'` 讓上游吃 SIGPIPE、`pipefail` 下整步 141；② `/usr/lib` 與 `/lib` 合併時 `ln` 連到自己。已修。
- draft Release 內容：三 distro 各有 `.deb`＋`.sig`、headless `tar.gz`＋`.sig`＋`.sha256`、離線安裝包 `cix3752iSorter-0.1.0-<distro>.tar.gz`（20.04 那份 90MB 含 webkit 棧），`latest.json` 六個平台鍵齊全。
- **待主人**：到 Releases 頁把 v0.1.0 由 draft 改為公開（自動更新才讀得到 `latest.json`），再把 `cix3752iSorter-0.1.0-ubuntu-20.04.tar.gz` 帶去正式機照 `docs/cutover.md` 做。

### 下一步：M6 現場切換

1. 主人在正式機實裝 GHA 產出的 `cix3752iSorter-0.1.0-ubuntu-20.04.tar.gz`（`sudo bash install.sh desktop` 或 `systemd`），預設埠 18090 已避開舊系統的 8080
2. ~~設定轉換腳本~~ 不需要：`config/mod.rs` 的預設值與 `migrations/0001_init.sql` 的格口初值就是現場 `conf.json`／`gkconfig.json`／舊 `chute` 表的值，首次啟動自動產生的設定即可用（網頁埠預設 18090，不會撞到舊系統的 8080）
3. ~~IR 光電檢查頁~~、~~supervisor 切換與回退步驟~~（都完成，見 `docs/cutover.md`）；剩：推 GitHub 跑發版、正式機實裝、實印一張對比、`p1` 格式確認
