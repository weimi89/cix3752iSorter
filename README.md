# 智配通 分揀控制（cix3752iSorter）

> 分揀機工控機上的單一執行檔：接皮帶線、分揀機、相機讀碼站，向中介機（`cix3752iLabelPrint`）取格口與面單，下分揀指令、列印面單、追蹤每一件包裹並提供網頁後台。取代廠商交付的 `main_proj`（Go `ecs1000` + Node `twfilter/server.js`）。

## 技術棧

- Rust：Tauri 2（桌面視窗）、tokio、axum、sqlx（SQLite）、reqwest、`image`
- 前端：Vue 3 + Vuetify + Pinia，`vite build` 後內嵌進二進位（作法對齊 `cix3752iLabelPrint/src-tauri/src/server/assets.rs`）；同一份前端在 Tauri 視窗與瀏覽器都能跑
- 兩種執行模式：預設開桌面視窗；`--headless`（或環境變數 `CIX_HEADLESS=1`）只跑服務，給 supervisor／systemd 用
- 目標平台：Ubuntu 20.04 / 22.04 / 24.04 x86_64，三個 distro 各出一份 .deb 與 headless 執行檔（20.04 沒有 webkit2gtk-4.1，發版時自編整套棧一起打包）

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
cargo run -- --headless --config config.toml --data-dir data   # 本機以服務模式跑（macOS 也能，裝置連不上會持續重連）
yarn tauri dev                                # 本機開桌面視窗（Vite :5180 熱更新）
yarn tauri build                              # 本機打 macOS 版；Linux 版只能由 GHA 建（見下）
```

**Linux 版只走 GitHub Actions**：Tauri 要連目標平台的 gtk／webkit 開發套件，macOS 交叉編譯不出來。
`release.yml` 在 ubuntu:20.04／22.04／24.04 三個 container 各建一份，每個 distro 上傳：
`cix3752iSorter-<ver>-<distro>.tar.gz`（離線安裝包：.deb + `install.sh` + 服務範本；20.04 另含自編 webkit 棧 `stack/`）、
`cix3752i-sorter_<ver>_<distro>_amd64.deb`（+ `.sig`）、`cix3752i-sorter_<ver>_<distro>_headless.tar.gz`（+ `.sig`／`.sha256`），最後合併出一份 `latest.json`。
20.04 那份第一次要自編 webkit（約 1 小時 13 分，之後走 Actions 快取；`warm-focal-cache.yml` 每週兩次刷新快取避免過期）。

工控機上：`tar -xzf cix3752iSorter-<ver>-<distro>.tar.gz && cd cix3752iSorter-<ver>-<distro> && sudo bash install.sh <模式>`，一台只選一種：

| 模式 | 裝法 | 之後怎麼升級 |
|---|---|---|
| `desktop` | 裝 .deb，從應用選單開視窗（20.04 先把 `stack/` 放進 /usr/local） | 程式內「發現新版本」→ Tauri updater 下載 .deb 安裝（需系統密碼，pkexec） |
| `systemd`／`supervisor` | 執行檔放安裝帳號家目錄下的 `cix3752iSorter/`，以 `--headless` 拉起 | 網頁後台「立即更新」→ 後端下載 headless tar.gz、校驗 SHA-256、換檔、交給 systemd／supervisor 重啟；沒外網可上傳 tar.gz |

設定檔 `config.toml` 首次啟動自動建立（桌面模式在使用者的應用資料夾，headless 在 `--config` 指定處），資料在 `data/`。
自動更新兩條路讀同一份 `latest.json`，`platforms` 的鍵帶 distro（`linux-x86_64-ubuntu-20.04`、`…-headless`），程式照 `/etc/os-release` 挑自己那筆。

發版：改 `src-tauri/Cargo.toml` 與 `src-tauri/tauri.conf.json` 版本（兩處要一致，GHA 會擋） → `git tag -a vX.Y.Z -m "版本說明"` → push tag → GHA 建置並上傳到 draft Release → 到 Releases 頁公開。
GitHub repo 的 secrets 要有 `TAURI_SIGNING_PRIVATE_KEY`／`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（對應 `~/.tauri/cix3752iSorter.key`），沒設 build 會直接失敗。

## 狀態

里程碑進度與待辦見 `docs/handover.md`；架構與里程碑定義見 `docs/plan.md`。
