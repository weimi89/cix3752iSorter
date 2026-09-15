# 舊系統（main_proj）分析與重寫依據

> 2026-09-14 分析。舊系統位於 `/Users/RD-CAT/Documents/喵_程式/main_proj`，正式機路徑 `/home/chipsort/main_proj`（Ubuntu 20.04 x86_64）。

## 1. 舊系統組成

| 行程 | 職責 | 連線 |
|---|---|---|
| Go `ecs1000`（gin） | 皮帶／分揀機控制、包裹追蹤、SQLite 歷史、網頁後台 `:8080` | TCP client → 皮帶 `192.168.177.100:10006`、分揀機 `192.168.177.198:10006`；TCP server `:8050` 收條碼；HTTP → Node `:3000/get/to/chute` |
| Node `twfilter/app/server.js` | 相機條碼挑選、問中介機、面單下載、Sharp 轉點陣、USB TSPL 列印、卡件／印表機警報 | `:8051` 收相機、`:3000` HTTP、`:3001` WS；HTTP → 中介機 `192.168.0.37:18080` |
| 中介機 | = 自家 `cix3752iLabelPrint`（Tauri v2 + axum） | `GET /api/parcel/{code}`、`POST /api/report`、`POST /api/device-alert` |
| `LogisticsCatPrinter` | Tauri 1 小工具：編輯 `gkconfig.json`、按鈕重啟 | 新系統有設定頁後可退役 |

## 2. 關鍵事實

1. **資料夾內 Go 原始碼不是正式機二進位的來源。** 二進位含 `web/chute.go`、`ChuteInfo` 表、`/setting/chute/*`、繁中日誌；原始碼沒有。`cmd.log` 的格式與舊原始碼一致，故架構相同、只是缺了較新的修改。已向廠商索取正式機完整原始碼。
2. 正式機 2026-08-26 ~ 09-10 共 59,879 件；重啟 12 次（09-07 一晚 3 次）；現場用 `c4000.sh`（`supervisorctl restart`）當日常操作。

## 3. 已驗證的根因

| 症狀 | 根因 | 證據 |
|---|---|---|
| 歷史詳情找不到 | 狀態 7（取消）／8（異常）的 `wcs_pac_detail.endTs` 寫 0，啟動時 `DELETE endTs < 15天前` 全刪 | 缺詳情 472 筆 = 372（狀態 7）+ 100（狀態 8）；正常件 0 筆缺 |
| 分到預設口 | 格口回應趕不上 `~O`（預算 ≈1.1s，中介機 p99 898ms + Node 1200ms 逾時） | 每日 0.3–0.9% 「返回超時」；`NoRead` 7% |
| 重啟就丟件 | 包裹只在結束時落 DB，在途件不持久化 | `pac.go:444` |

## 4. 舊程式結構性缺陷（新版必須避免）

| 缺陷 | 舊碼位置 | 新版對策 |
|---|---|---|
| 斷線時 `Conn=nil` 仍解參考 → panic | `sorter.go:196`、`belt.go:147` | 連線句柄只在裝置 task 內持有，外部經 channel 送指令 |
| 重連後舊控制器 goroutine 未停 → 重複控制器 | `sorter.go:75-120` | 每條連線一個 `CancellationToken`，斷線先 cancel 再重連 |
| RLock 未釋放 → 永久死鎖 | `wis.go:154-218` | 狀態機單一 owner task，無共享鎖 |
| 皮帶控制器同步打 HTTP 2 秒 | `belt.go:223` | 格口查詢非同步，`~O` 到時取已就緒答案 |
| 無 read deadline／keepalive | 全部 TCP | 讀逾時 + TCP keepalive + 應用層心跳（皮帶 `~k-1` 可當心跳） |
| 陣列越界（cart id 固定 26） | `sorter.go:245` | 以 map 儲存、輸入驗證 |
| DB 錯誤全吞、goroutine 寫入 | `pac.go:475` | sqlx 錯誤上報事件 log；建立即落 DB |
| 收件號 = 啟動 unix 秒 + 累加 | `pac.go:17` | DB 自增 id + 業務用 ULID |
| SQL 字串拼接 | `web/history.go:33-62` | 參數化 |
| 印表機寫入佔滿 libuv 執行緒池 | `server.js:1077` | 每台印表機一個 `spawn_blocking` worker，佇列落 SQLite |

## 5. 現場資料

- `main_proj/data/cmd.log`：71 萬行真實訊號，作為裝置模擬器回放素材。
- `main_proj/data/info.log`：格口查詢延遲統計來源。
- `main_proj/data/sortAi.db`：舊表結構與 `chute` 對照。
- `main_proj/gkconfig.json`：8 台印表機 USB bus-port 對照（`1-3.1`…`1-8.4`）與 `print_profile`。
- `main_proj/data/conf.json`：裝置位址、指令字串、NG 規則、燈號。
