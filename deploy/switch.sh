#!/bin/bash
# 新舊分揀程式一鍵切換（deploy/switch.sh，由 install.sh 裝成 /usr/local/bin/sorter-switch）。
#
#   sorter-switch to-new    停舊程式（main_proj、twfilter）→ 確認裝置與埠位放掉 → 舊的改成不自啟 → 起新程式 → 等它連上裝置
#   sorter-switch to-old    停新程式 → 拿掉自動啟動 → 舊程式設定改回來並啟動 → 等舊後台回來
#   sorter-switch status    現在誰在跑
#
# 用登入的那個帳號跑（不要整支 sudo）：要用你的桌面把視窗開出來，需要 root 的步驟會自己 sudo。
# 新舊程式搶同一台相機、同一台分揀機與印表機，所以一定是先停一邊再起另一邊，中間會確認埠位真的放掉才往下走。
set -euo pipefail

OLD_PROGRAMS="main_proj twfilter"
OLD_INI_DIR="/etc/supervisor/conf.d"
NEW_PORT="${SORTER_PORT:-18090}"
OLD_PORT=8080
CAMERA_PORT=8051

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m✗\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "缺少指令 $1"; }
need supervisorctl; need curl; need ss; need pgrep

# 自動啟動的 .desktop 檔名是程式顯示名稱（中文，含空白），用內容的 Exec 行找比較穩；
# 輸出用 NUL 分隔，後面一律接 xargs -0——用空白切會把檔名切成兩半、刪不掉
autostart_files() { grep -lsZ 'sorter' "$HOME"/.config/autostart/*.desktop 2>/dev/null || true; }
# 管線後面不用 grep -q：它讀到就關管線，前面的指令會吃 SIGPIPE，pipefail 下整個判斷變成假的
old_running()    { sudo supervisorctl status 2>/dev/null | grep -E "^(main_proj|twfilter) +RUNNING" >/dev/null; }
new_running()    { pgrep -x sorter >/dev/null; }
port_listening() { ss -ltn "( sport = :$1 )" 2>/dev/null | tail -n +2 | grep . >/dev/null; }
# 只數建立中的連線：程式剛結束時 socket 還會在 FIN_WAIT／TIME_WAIT 留一分鐘，算進去會誤判成「還連著」
device_conns()   { ss -Htn state established '( dport = :10006 )' 2>/dev/null | grep -c . || true; }

# 等到條件成立或逾時；$1 秒數、$2 說明、其餘是要成立的指令
wait_for() {
  local secs=$1 what=$2; shift 2
  local i=0
  while ! "$@"; do
    i=$((i + 1))
    [ "$i" -ge "$secs" ] && return 1
    printf '   等 %s… %ds\r' "$what" "$i"
    sleep 1
  done
  printf '\n'
}

show_status() {
  say "舊程式（supervisor）"
  sudo supervisorctl status 2>/dev/null | grep -E "main_proj|twfilter" || echo "   （supervisor 沒有登記舊程式）"
  say "新程式"
  if new_running; then echo "   執行中"; else echo "   沒在跑"; fi
  say "埠位"
  for p in $CAMERA_PORT 8050 $OLD_PORT "$NEW_PORT"; do
    # 舊程式以 root 跑，程序名要 sudo 才看得到
    if port_listening "$p"; then echo "   :$p 有人在聽（$(sudo ss -ltnp "( sport = :$p )" 2>/dev/null | tail -n +2 | grep -o 'users:(("[^"]*"' | head -1 | cut -d'"' -f2)）"; else echo "   :$p 空"; fi
  done
  say "與皮帶／分揀機（10006）的連線數：$(device_conns)"
  if old_running && new_running; then
    warn "新舊程式同時在跑，兩邊都在對皮帶／分揀機下指令！立刻決定一邊：sorter-switch to-new 或 sorter-switch to-old"
  fi
  if [ -n "$(autostart_files)" ]; then say "登入後自動啟動：已登記"; else say "登入後自動啟動：沒有"; fi
  say "舊程式設定檔"
  ls -1 "$OLD_INI_DIR"/main_proj.ini* "$OLD_INI_DIR"/twfilter.ini* 2>/dev/null | sed 's|^|   |' || true
}

NEW_LAUNCH_LOG="$HOME/.local/share/com.weiminet.cix3752i.sorter/data/logs/sorter-launch.log"

# 借登入中桌面工作階段的顯示環境：從 SSH 或沒有桌面的終端機跑時沒有 DISPLAY，
# 視窗程式會在還沒寫日誌前就結束，什麼痕跡都不留
borrow_desktop_env() {
  local pid
  pid=$(pgrep -u "$USER" -x gnome-shell | head -1 || true)
  [ -n "$pid" ] || die "這個終端機沒有 DISPLAY，也找不到 $USER 登入中的桌面：請先在工控機登入桌面，或直接在桌面的終端機執行"
  while IFS= read -r -d '' kv; do
    case "$kv" in DISPLAY=*|XAUTHORITY=*|DBUS_SESSION_BUS_ADDRESS=*|XDG_SESSION_TYPE=*|WAYLAND_DISPLAY=*|XDG_RUNTIME_DIR=*) export "$kv" ;; esac
  done < "/proc/$pid/environ"
  [ -n "${DISPLAY:-}" ] || die "登入中的桌面（gnome-shell pid $pid）沒有 DISPLAY，無法開視窗"
  warn "這裡不是桌面終端機，借用登入桌面的顯示環境開視窗（DISPLAY=$DISPLAY）"
}

start_new() {
  command -v sorter >/dev/null || die "找不到 sorter，先跑 sudo bash install.sh"
  [ -n "${DISPLAY:-}" ] || borrow_desktop_env
  mkdir -p "$(dirname "$NEW_LAUNCH_LOG")"
  # 從使用者的桌面工作階段開，關掉這個終端機也不會跟著關；啟動期的輸出留檔，起不來時第 4 步會印出來
  setsid nohup sorter > "$NEW_LAUNCH_LOG" 2>&1 < /dev/null &
}

stop_new() {
  pkill -TERM -x sorter 2>/dev/null || true
}

to_new() {
  if new_running && ! old_running; then say "已經是新程式在跑，不用切"; show_status; return; fi

  # 新程式若已經開著（例如有人先從選單點開來看），舊程式一放手它就會接上皮帶／分揀機，
  # 下面「裝置連線都放掉」的檢查永遠過不了；先關掉，第 4 步再乾淨地重開
  if new_running; then
    say "新程式已在跑，先關掉，等舊程式放手後再重開"
    stop_new
    wait_for 20 "新程式結束" bash -c "! pgrep -x sorter >/dev/null" || { warn "新程式沒在 20 秒內結束，強制關閉"; pkill -KILL -x sorter 2>/dev/null || true; sleep 1; }
  fi

  say "1/5 停舊程式"
  # shellcheck disable=SC2086
  sudo supervisorctl stop $OLD_PROGRAMS 2>/dev/null | sed 's|^|   |' || true

  say "2/5 確認舊程式結束、相機埠與裝置連線都放掉"
  # 樣式用 [e] 這種寫法：pgrep -f 會連跑這條檢查的 bash -c 自己的命令列一起比對，寫成純字串會永遠「還在」
  wait_for 20 "舊程式行程結束" bash -c "! pgrep -f '[e]cs1000|[t]wfilter/app/server.js' >/dev/null" || die "舊程式的行程還在：$(pgrep -fl '[e]cs1000|[t]wfilter/app/server.js' | tr '\n' ' ')；先看 sudo supervisorctl status"
  wait_for 20 "相機埠 :$CAMERA_PORT 釋放" bash -c "! ss -ltn '( sport = :$CAMERA_PORT )' | tail -n +2 | grep -q ." || die "舊 Node 還占著 :$CAMERA_PORT，先看 sudo supervisorctl status"
  wait_for 20 "分揀機／皮帶連線關閉" bash -c "[ \"\$(ss -Htn state established '( dport = :10006 )' | grep -c .)\" -eq 0 ]" || die "還有程式連著分揀機或皮帶（:10006）：$(ss -Htnp state established '( dport = :10006 )' 2>/dev/null | tr '\n' ' ')"

  say "3/5 舊程式改成開機不自啟（改副檔名，回退時改回來）"
  for p in $OLD_PROGRAMS; do
    [ -f "$OLD_INI_DIR/$p.ini" ] && sudo mv "$OLD_INI_DIR/$p.ini" "$OLD_INI_DIR/$p.ini.off"
  done
  sudo supervisorctl reread >/dev/null && sudo supervisorctl update >/dev/null

  say "4/5 啟動新程式"
  start_new
  wait_for 40 "新程式網頁服務 :$NEW_PORT" bash -c "curl -sf http://127.0.0.1:$NEW_PORT/api/health >/dev/null" || {
    warn "新程式啟動輸出（$NEW_LAUNCH_LOG）最後幾行："
    tail -n 15 "$NEW_LAUNCH_LOG" 2>/dev/null | sed 's|^|   |'
    die "新程式 40 秒內沒起來；看上面的輸出或 ~/.local/share/com.weiminet.cix3752i.sorter/data/logs/，要回退執行 sorter-switch to-old"
  }

  say "5/5 等裝置連上（最多 15 秒）"
  local i=0 st
  while [ $i -lt 15 ]; do
    st=$(curl -sf "http://127.0.0.1:$NEW_PORT/api/status" | python3 -c 'import json,sys; d=json.load(sys.stdin)["devices"]; print(" ".join(k+"="+("連線" if v.get("connected") else "未連線") for k,v in d.items()))' 2>/dev/null || echo "")
    case "$st" in *未連線*|"") i=$((i + 1)); sleep 1 ;; *) break ;; esac
  done
  echo "   ${st:-讀不到狀態}"
  case "$st" in
    *未連線*|"") warn "有裝置還沒連上，看看板；30 秒內沒亮就 sorter-switch to-old 回退" ;;
    *) say "切換完成。接著照 docs/cutover.md 第 2 節的驗證清單跑一遍（掃一件、印一張）" ;;
  esac
}

to_old() {
  say "1/4 停新程式"
  stop_new
  wait_for 20 "新程式放掉埠位" bash -c "! ss -ltn '( sport = :$CAMERA_PORT )' | tail -n +2 | grep -q . && ! pgrep -x sorter >/dev/null" || { warn "新程式沒在 20 秒內結束，強制關閉"; pkill -KILL -x sorter 2>/dev/null || true; sleep 1; }

  say "2/4 拿掉新程式的登入後自動啟動（不然重開機又會跳出來搶裝置）"
  autostart_files | xargs -0 -r rm -f
  if [ -n "$(autostart_files)" ]; then die "自動啟動檔沒刪掉：$(autostart_files | tr '\0' ' ')；手動刪掉再重開機，否則新程式會再跳出來搶裝置"; fi

  say "3/4 舊程式設定改回來並啟動"
  for p in $OLD_PROGRAMS; do
    [ -f "$OLD_INI_DIR/$p.ini.off" ] && sudo mv "$OLD_INI_DIR/$p.ini.off" "$OLD_INI_DIR/$p.ini"
  done
  sudo supervisorctl reread >/dev/null && sudo supervisorctl update >/dev/null
  # shellcheck disable=SC2086
  sudo supervisorctl start $OLD_PROGRAMS 2>/dev/null | sed 's|^|   |' || true

  say "4/4 等舊後台 :$OLD_PORT 回來"
  wait_for 40 "舊後台" bash -c "curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$OLD_PORT/ | grep -q 200" || die "舊後台 40 秒內沒回來，看 sudo supervisorctl status 與 /etc/supervisor/main_proj.log"
  say "已回退到舊程式"
  sudo supervisorctl status | grep -E "main_proj|twfilter" | sed 's|^|   |'
}

case "${1:-}" in
  to-new) to_new ;;
  to-old) to_old ;;
  status) show_status ;;
  *) echo "用法: sorter-switch to-new | to-old | status"; exit 1 ;;
esac
