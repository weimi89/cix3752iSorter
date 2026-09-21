# 智配通 分揀控制（cix3752iSorter）

> 分揀機工控機上的單一執行檔：接皮帶線、分揀機、相機讀碼站，向中介機（`cix3752iLabelPrint`）取格口與面單，下分揀指令、列印面單、追蹤每一件包裹並提供網頁後台。取代廠商交付的 `main_proj`（Go `ecs1000` + Node `twfilter/server.js`）。

## 技術棧

- Rust：Tauri 2（桌面視窗）、tokio、axum、sqlx（SQLite）、reqwest、`image`
- 前端：Vue 3 + Vuetify + Pinia，`vite build` 後內嵌進二進位（作法對齊 `cix3752iLabelPrint/src-tauri/src/server/assets.rs`）；同一份前端在 Tauri 視窗與瀏覽器都能跑
- 桌面程式：開視窗、登入後自動啟動；`--headless` 只給開發機接模擬器用，現場不用
- 目標平台：工控機 Ubuntu 20.04 x86_64（20.04 沒有 webkit2gtk-4.1，發版時自編整套棧一起打包）；22.04／24.04 的建置矩陣保留，升級時再開

## 文件

文件集中在私有的 `cix3752iBrain` repo，不隨本專案發布。

| 文件 | 內容 |
|---|---|
| `../cix3752iBrain/docs/cix3752iSorter/legacy-analysis.md` | 舊系統組成、已驗證根因、結構性缺陷與對策 |
| `../cix3752iBrain/docs/cix3752iSorter/protocol-spec.md` | 皮帶／分揀機／相機／印表機協定與實測時序（由 71 萬行現場日誌逆推） |
| `../cix3752iBrain/docs/cix3752iSorter/plan.md` | 架構決策、模組切分、資料表、API、里程碑 |
| `../cix3752iBrain/docs/cix3752iSorter/cutover.md` | 現場切換與回退步驟（停舊 → 起新、驗證清單、2 分鐘回退） |
| `../cix3752iBrain/docs/cix3752iSorter/handover.md` | 進度與交接 |
| `../cix3752iBrain/docs/cix3752iLabelPrint/local-http-api.md` | 中介機 API 契約（`/api/parcel`、`/api/report`、`/api/device-alert`） |

## 網頁後台的存取控制

同一份畫面在桌面視窗、現場電腦的瀏覽器、手機（`/control` 遙控頁）都能開；**從現場網路以外連進來要輸入共用密碼**（`src-tauri/src/server/auth.rs`）。

| 來源 | 待遇 |
|---|---|
| 工控機本機、`web_access.lan_cidrs` 內的網段（預設涵蓋所有私有網段） | 免登入，完整權限 |
| 其他來源 | 「開放外部連線」關閉時（預設）一律 403，連登入頁都沒有；開啟後要輸入密碼，登入後與現場同權限 |

- 密碼在「系統設定 → 網頁存取」設定，至少 8 個字，argon2 雜湊存 SQLite `app_setting`，**不在** `config.toml`。
- 改網頁存取設定與換密碼只能在本機或現場網路內做（拿到密碼的人不能把門鎖換掉）。
- 同一來源密碼連錯 `max_fail_attempts` 次鎖 `lock_minutes` 分鐘；登入成功／失敗／被拒都寫進事件記錄的「security」類。
- 內外網只認 TCP 對端位址，不看 `X-Forwarded-For`；跨站請求（`Sec-Fetch-Site`／`Origin` 不符）一律擋，桌面視窗的 `tauri://localhost` 例外。
- **前提是路由器直接 Port Forward 到工控機**。若日後在前面擺反向代理或 tunnel（nginx、Cloudflare Tunnel、VPN 閘道），後端看到的來源會變成代理的位址而被當成內網、整道門失效——要先改 `auth.rs` 只信任該代理覆寫的來源欄位，不可直接部署。
- **尚未加 TLS**：外網那段是明文，只建議需要時才開。

## 讀碼站照片（收件證據）

讀碼器（海康 MV-ID6200M）每件都拍一張，設定成用 FTP 上傳到本程式（`src-tauri/src/device/camera_ftp.rs`），
當「這件我們有收到」的證據；包裹詳情、異常口處理、包裹查詢都看得到。

- 本程式自己當 FTP 伺服器（預設 `0.0.0.0:2121`、帳密 `sorter`／`sorter`），只收 `STOR`，不給下載、刪檔、列目錄；PASV 與 PORT 兩種資料連線都接。
- 收到原圖立刻縮成證據圖（長邊 1600px、JPEG 80，約 150–250 KB）存 `data/images/YYYY-MM-DD/`，原圖不留；讀碼失敗的件另留原圖在 `orig/`。20MP 原圖一張 2–3 MB，一天五千件全留會把硬碟吃光。
- 照片對回包裹：檔名含窗口內某件的條碼就對那件；否則取窗口內（預設 5 秒）最早、還沒有照片的件。對不到的照片仍存檔並記事件。
- 保留天數獨立（`camera_ftp.retention_days`，預設 90），與包裹資料的 15 天分開；每小時清一次。
- 存放目錄可改（`camera_ftp.images_dir`，絕對路徑；空白 = `data/images`），要跟資料庫分開放到另一顆硬碟就填這裡；換目錄不搬舊圖。
- 畫面上照片與資料分開：照片按鈕／縮圖直接開 `ParcelImageViewer`（標題帶條碼、格口、時間；`GET /api/parcel-images/{id}` 給 meta），包裹詳情只有資料、流程里程碑與白話時間軸。
- 畫面上的照片一律經 `ProtectedImg`（fetch → blob）顯示：桌面視窗是 `tauri://localhost` 跨來源，`<img src>` 子資源不帶 Origin 會被跨站防護擋成 403。
- 工控機開著 ufw：要放行監聽埠與被動模式埠範圍（預設 50000–50100），來源限相機。
- 讀碼器端（IDMVS）：6.通信配置 → FTP 主機填本機在相機網段的 IP、埠與帳密同上；5.數據處理存圖條件選全部、JPEG；改完存到用戶配置 1。

## 建置與部署

```bash
yarn install && yarn build                    # 前端（cargo 內嵌 ../dist；沒建過會用佔位頁）
cd src-tauri && cargo test                    # 單元測試
cargo run -- --headless --config config.toml --data-dir data   # 本機不開視窗只跑服務（接模擬器用；macOS 也能，裝置連不上會持續重連）
yarn tauri dev                                # 本機開桌面視窗（Vite :5180 熱更新）
yarn tauri build                              # 本機打 macOS 版；Linux 版只能由 GHA 建（見下）
```

**Linux 版只走 GitHub Actions**：Tauri 要連目標平台的 gtk／webkit 開發套件，macOS 交叉編譯不出來。
`release.yml` 只建現場用的 distro（目前 Ubuntu 20.04；22.04／24.04 的矩陣項目留在註解，工控機升級再開），Release 只放 3 個檔：
`cix3752iSorter-<ver>-<distro>.tar.gz`（離線安裝包：.deb + `install.sh` + 服務範本 + 自編 webkit 棧 `stack/`）、
`cix3752i-sorter_<ver>_<distro>_amd64.deb`（程式內自動更新用）、`latest.json`（簽章在裡面）。
20.04 那份第一次要自編 webkit（約 1 小時 55 分，之後走 Actions 快取；`warm-focal-cache.yml` 每週兩次刷新快取避免過期）。

工控機上：`tar -xzf cix3752iSorter-<ver>-<distro>.tar.gz && cd cix3752iSorter-<ver>-<distro> && sudo bash install.sh`（只裝不啟動；20.04 會先把 `stack/` 放進 /usr/local），
切換用 `sorter-switch to-new`、回退 `sorter-switch to-old`（步驟見 cix3752iBrain 的 `docs/cix3752iSorter/cutover.md`）。之後升級由程式內「發現新版本」處理（Tauri updater 裝 .deb，會要系統密碼）。

設定檔 `config.toml` 與資料在使用者的應用資料夾（`~/.local/share/com.weiminet.cix3752i.sorter/`），首次啟動自動建立。
`latest.json` 的 `platforms` 鍵帶 distro（`linux-x86_64-ubuntu-20.04`），程式照 `/etc/os-release` 挑自己那筆。

發版（與 cix3752iLabelPrint 同一套）：改 `src-tauri/Cargo.toml` 與 `src-tauri/tauri.conf.json` 版本（兩處要一致，GHA 會擋） → 在 `CHANGELOG.md` 新增 `## vX.Y.Z` 段落（Release 說明與程式內更新提示都從這裡抽） → `git tag -a vX.Y.Z -m "vX.Y.Z"` → push tag → GHA 建置並上傳到 draft Release → 到 Releases 頁公開。已公開後才發現說明漏寫：`gh release edit vX.Y.Z --notes-file <(bash scripts/build-release-notes.sh vX.Y.Z)`，不要靠重跑 workflow。
GitHub repo 的 secrets 要有 `TAURI_SIGNING_PRIVATE_KEY`／`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（對應 `~/.tauri/cix3752iSorter.key`），沒設 build 會直接失敗。

## 狀態

里程碑進度與待辦見 cix3752iBrain 的 `docs/cix3752iSorter/handover.md`；架構與里程碑定義見同目錄 `plan.md`。
