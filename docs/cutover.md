# 現場切換與回退步驟

> 目標：上線當天照著貼指令就能把舊程式換成新程式；出狀況時 **2 分鐘內**讓舊程式恢復出貨。
> 正式機：Ubuntu 20.04，舊程式由 supervisor 管理（設定抓回來放在 `main_proj/supervisor/`）。下面的 `~` 都是安裝帳號的家目錄。

## 0. 現況（切換前的樣子）

| 項目 | 舊程式 | 新程式 |
|---|---|---|
| supervisor 程式名 | `main_proj`（Go `ecs1000`，`autorestart=false`）、`twfilter`（Node `server.js`） | `cix3752i-sorter`（headless 模式才有） |
| 設定檔 | `/etc/supervisor/conf.d/main_proj.ini`、`twfilter.ini`（**只認 `.ini`**，`supervisord.conf` 的 include 是 `conf.d/*.ini`） | `/etc/supervisor/conf.d/cix3752i-sorter.ini` |
| 執行身分 | root | 安裝時執行 `sudo` 的那個帳號（會自動加進 `lp` 群組才能寫印表機） |
| 網頁後台 | `:8080` | `:18090` |
| 收相機 | Node `:8051`（Go `:8050` 收 Node 轉來的碼） | 直接 `:8051` |
| 連皮帶／分揀機 | `192.168.177.100:10006`、`192.168.177.198:10006` | 同 |
| 中介機 | `192.168.0.37:18080` | 同 |
| 日誌 | `/etc/supervisor/main_proj.log`、`twfilter.log` | `data/logs/sorter.*.log`、`signals-*.log`（網頁「事件記錄」可下載） |
| 資料 | `main_proj/data/sortAi.db` | `data/sorter.sqlite`（**互不相干**，回退不用動資料） |

**為什麼不能並存**：相機只會連到一個 8051；皮帶與分揀機同時被兩個程式下指令會亂；印表機 USB 同時寫會印壞。所以一定是「停舊 → 起新」，回退則「停新 → 起舊」。

## 1. 切換前一天（不影響現場）

```bash
# 1-1 把安裝包放到工控機並解開（版本號換成實際的）
cd ~ && tar -xzf cix3752iSorter-0.1.0-ubuntu-20.04.tar.gz && cd cix3752iSorter-0.1.0-ubuntu-20.04

# 1-2 安裝但先不要起服務：headless 模式用 supervisor（與舊程式同一套管理方式，回退最快）
sudo bash install.sh supervisor
sudo supervisorctl stop cix3752i-sorter        # 裝完會自動起來，先停掉，明天再切

# 1-3 確認新程式登記進去了（要看到 cix3752i-sorter STOPPED；看不到就是 .ini 沒被 include）
sudo supervisorctl status

# 1-4 預設設定就是現場值，只確認一眼
grep -E "bind|addr|base_url|listen" ~/cix3752iSorter/config.toml
```

預期：`bind = "0.0.0.0:18090"`、皮帶 `192.168.177.100:10006`、分揀機 `192.168.177.198:10006`、相機 `0.0.0.0:8051`、中介機 `http://192.168.0.37:18080/`。

> 桌面模式（有螢幕、要從應用選單開視窗）改跑 `sudo bash install.sh desktop`，第 2 節的「起新」改成從應用選單開「智配通 分揀控制」，其餘相同。**一台只選一種。**

## 2. 切換（線上沒有包裹時做，約 3 分鐘）

```bash
# 2-1 停舊：先停 Node（放掉相機 8051），再停 Go（放掉皮帶／分揀機連線）
sudo supervisorctl stop twfilter main_proj

# 2-2 確認埠位與連線都放掉了：三行都要是空的
sudo ss -ltnp | grep -E ':8051|:8050|:8080'
sudo ss -tnp | grep -E '192.168.177.100:10006|192.168.177.198:10006'
pgrep -fl 'ecs1000|twfilter/app/server.js'

# 2-3 讓舊程式開機不要再自己起來（照廠商停 sort_box 的做法：改副檔名）
sudo mv /etc/supervisor/conf.d/main_proj.ini /etc/supervisor/conf.d/main_proj.ini.off
sudo mv /etc/supervisor/conf.d/twfilter.ini  /etc/supervisor/conf.d/twfilter.ini.off
sudo supervisorctl reread && sudo supervisorctl update

# 2-4 起新
sudo supervisorctl start cix3752i-sorter
sudo supervisorctl status cix3752i-sorter      # RUNNING
curl -s http://127.0.0.1:18090/api/health       # {"ok":true,...}
```

### 切換後驗證（逐項打勾，全部過才算切換完成）

| # | 看什麼 | 怎麼看 | 沒過怎麼辦 |
|---|---|---|---|
| 1 | 皮帶線、分揀機、讀碼站三個燈 **全綠** | 瀏覽器開 `http://<工控機IP>:18090` 看板 | 皮帶／分揀機紅：舊程式沒放掉連線，回 2-2 查；讀碼站紅：相機還沒重連（等 10 秒）或舊 Node 沒停 |
| 2 | 皮帶能啟停 | 看板右上角按「啟動皮帶」→ 狀態變「運轉中」 | 訊號日誌看 `belt > KM998 3` 有沒有送出 |
| 3 | 掃一件包裹跑完 | 放一件上線：看板「目前處理中」出現條碼 → 格口對 → 「完成」+1 | 走預設口：看板「格口查詢」卡是否超預算、系統訊息有沒有「面單下載失敗」 |
| 4 | 面單印得出來、掃得過 | 同一件的面單從對應格口印表機出來，用手機掃條碼 | 沒印：「列印任務」頁看狀態與錯誤；印出但掃不過：見 `docs/handover.md` 縮放差異，回退 |
| 5 | 中介機有收到回報 | 中介機（LabelPrint）看板該件狀態變已回報 | 「回報佇列」頁看是否卡 pending／failed |
| 6 | 光電檢查有回應 | 「光電檢查」頁按重新檢查，8 格都有顏色 | 灰 = 分揀機沒回 `~[`，看訊號日誌 |
| 7 | 跑 10 件以上看走預設口比例 | 看板「走預設口」不應明顯高於舊系統（每日 0.3–0.9%） | 高：「格口查詢」卡 p99 是否 > 1100ms |

## 3. 回退（出狀況時，目標 2 分鐘）

```bash
# 3-1 停新
sudo supervisorctl stop cix3752i-sorter

# 3-2 舊程式設定復原並拉起（先 Go 再 Node，與開機順序相同）
sudo mv /etc/supervisor/conf.d/main_proj.ini.off /etc/supervisor/conf.d/main_proj.ini
sudo mv /etc/supervisor/conf.d/twfilter.ini.off  /etc/supervisor/conf.d/twfilter.ini
sudo supervisorctl reread && sudo supervisorctl update
sudo supervisorctl start main_proj twfilter
sudo supervisorctl status                       # main_proj、twfilter 都 RUNNING

# 3-3 確認舊後台回來
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/   # 200
```

桌面模式的回退：3-1 改成關閉「智配通 分揀控制」視窗（關視窗＝整個程式結束，裝置連線會乾淨放掉），其餘相同。

回退後**不用動任何資料**：舊程式讀自己的 `sortAi.db`，新程式的 `data/` 留著，之後查問題用。

### 什麼情況要回退（不要猶豫）

| 症狀 | 判斷 |
|---|---|
| 三個燈有一個 2 分鐘內亮不起來 | 回退，事後查 `data/logs/sorter.*.log` 的連線錯誤 |
| 連續 3 件面單印不出來或掃不過 | 回退（面單掃不過會影響下游） |
| 分揀機一直「Kn 後收不到 ~c」停線 | 回退，可能是指令格式與正式機不符 |
| 10 分鐘內走預設口比例 > 5% | 回退，先看中介機是不是變慢 |
| 看板數字不動、事件記錄一直斷線重連 | 回退 |

## 4. 切換後的收尾（跑穩一個班次之後）

- 舊程式的 `.ini.off` 保留一週，確定不回退再刪；`main_proj/` 資料夾整個留著（回退還要用）。
- 桌面上的「C4000」圖示（`c4000.sh`，會 `supervisorctl restart main_proj`）要拿掉或改成重啟 `cix3752i-sorter`，否則現場人員按下去會把舊程式拉起來搶相機。
- supervisor 的網頁介面（`:8082`）沒動，新程式也會出現在裡面。

## 附：常用查看指令

```bash
sudo supervisorctl status                                   # 三支程式誰在跑
sudo tail -f /var/log/supervisor/cix3752i-sorter.log        # 新程式 stdout
tail -f ~/cix3752iSorter/data/logs/signals-$(date +%F).log   # 裝置原始訊號
sudo tail -f /etc/supervisor/main_proj.log                  # 舊程式（回退後）
```
