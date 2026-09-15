# 智配通 分揀控制（cix3752iSorter）

> 分揀機工控機上的單一執行檔：接皮帶線、分揀機、相機讀碼站，向中介機（`cix3752iLabelPrint`）取格口與面單，下分揀指令、列印面單、追蹤每一件包裹並提供網頁後台。取代廠商交付的 `main_proj`（Go `ecs1000` + Node `twfilter/server.js`）。

## 技術棧

- Rust：Tauri 2（桌面視窗）、tokio、axum、sqlx（SQLite）、reqwest、`image`
- 前端：Vue 3 + Vuetify + Pinia，`vite build` 後內嵌進二進位（作法對齊 `cix3752iLabelPrint/src-tauri/src/server/assets.rs`）；同一份前端在 Tauri 視窗與瀏覽器都能跑
- 桌面程式：開視窗、登入後自動啟動；`--headless` 只給開發機接模擬器用，現場不用
- 目標平台：工控機 Ubuntu 20.04 x86_64（20.04 沒有 webkit2gtk-4.1，發版時自編整套棧一起打包）；22.04／24.04 的建置矩陣保留，升級時再開

## 文件

| 文件 | 內容 |
|---|---|
| [`docs/legacy-analysis.md`](docs/legacy-analysis.md) | 舊系統組成、已驗證根因、結構性缺陷與對策 |
| [`docs/protocol-spec.md`](docs/protocol-spec.md) | 皮帶／分揀機／相機／印表機協定與實測時序（由 71 萬行現場日誌逆推） |
| [`docs/plan.md`](docs/plan.md) | 架構決策、模組切分、資料表、API、里程碑 |
| [`docs/cutover.md`](docs/cutover.md) | 現場切換與回退步驟（停舊 → 起新、驗證清單、2 分鐘回退） |
| [`docs/handover.md`](docs/handover.md) | 進度與交接 |
| `cix3752iLabelPrint/docs/local-http-api.md` | 中介機 API 契約（`/api/parcel`、`/api/report`、`/api/device-alert`） |

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
切換用 `sorter-switch to-new`、回退 `sorter-switch to-old`（步驟見 `docs/cutover.md`）。之後升級由程式內「發現新版本」處理（Tauri updater 裝 .deb，會要系統密碼）。

設定檔 `config.toml` 與資料在使用者的應用資料夾（`~/.local/share/com.weiminet.cix3752i.sorter/`），首次啟動自動建立。
`latest.json` 的 `platforms` 鍵帶 distro（`linux-x86_64-ubuntu-20.04`），程式照 `/etc/os-release` 挑自己那筆。

發版（與 cix3752iLabelPrint 同一套）：改 `src-tauri/Cargo.toml` 與 `src-tauri/tauri.conf.json` 版本（兩處要一致，GHA 會擋） → 在 `CHANGELOG.md` 新增 `## vX.Y.Z` 段落（Release 說明與程式內更新提示都從這裡抽） → `git tag -a vX.Y.Z -m "vX.Y.Z"` → push tag → GHA 建置並上傳到 draft Release → 到 Releases 頁公開。已公開後才發現說明漏寫：`gh release edit vX.Y.Z --notes-file <(bash scripts/build-release-notes.sh vX.Y.Z)`，不要靠重跑 workflow。
GitHub repo 的 secrets 要有 `TAURI_SIGNING_PRIVATE_KEY`／`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（對應 `~/.tauri/cix3752iSorter.key`），沒設 build 會直接失敗。

## 狀態

里程碑進度與待辦見 `docs/handover.md`；架構與里程碑定義見 `docs/plan.md`。
