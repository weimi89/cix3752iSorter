#!/bin/bash
# 工控機安裝／升級（解開發版的 cix3752iSorter-<ver>-<distro>.tar.gz 後於該目錄執行）。
#
#   sudo bash install.sh desktop      # 桌面模式：裝 .deb，從應用選單開視窗；之後由程式內建的更新升級（會跳系統密碼）
#   sudo bash install.sh systemd      # 無畫面：執行檔放 <安裝帳號家目錄>/cix3752iSorter，由 systemd 以 --headless 拉起
#   sudo bash install.sh supervisor   # 同上，改用現場既有的 supervisor 管理
#
# 一台機器只選一種：桌面與 headless 都綁同一個埠，兩種都裝會互搶。
# 只換程式，不動 config.toml 與 data/；第一次啟動時由程式自建預設設定檔。
#
# 安裝包若含 stack/（Ubuntu 20.04 版），代表這個 distro 沒有 webkit2gtk-4.1，會先把自編的
# glib／libsoup3／webkit 放進 /usr/local；日後只升程式（.deb 或程式內更新）不必重跑這段，
# 但發版說明寫「webkit 棧有更新」時就要重跑完整安裝包。
set -euo pipefail

MODE="${1:-desktop}"
# 服務以哪個帳號跑、裝在哪：預設是執行 sudo 的那個人與他的家目錄；要換就 APP_USER=xxx sudo -E bash install.sh …
APP_USER="${APP_USER:-${SUDO_USER:-$(id -un)}}"
APP_HOME="$(getent passwd "${APP_USER}" | cut -d: -f6)"
[ -n "${APP_HOME}" ] || { echo "找不到帳號 ${APP_USER} 的家目錄"; exit 1; }
APP_DIR="${APP_DIR:-${APP_HOME}/cix3752iSorter}"
HERE="$(cd "$(dirname "$0")" && pwd)"

# 範本裡的 __APP_USER__／__APP_DIR__ 在安裝時才代入，repo 裡不留現場帳號
render() { sed -e "s|__APP_USER__|${APP_USER}|g" -e "s|__APP_DIR__|${APP_DIR}|g" "$1"; }

DEB="$(find "${HERE}" -maxdepth 1 -name 'cix3752i-sorter_*.deb' -print -quit)"
[ -n "${DEB}" ] || { echo "找不到 cix3752i-sorter_*.deb，請在解開的安裝包目錄執行"; exit 1; }

case "${MODE}" in
  desktop|systemd|supervisor) ;;
  *) echo "用法: sudo bash install.sh [desktop|systemd|supervisor]"; exit 1 ;;
esac

echo "=== 智配通 分揀控制 — 安裝（${MODE}）==="

# 1. 自編 webkit 棧（只有 20.04 的包有）
if [ -d "${HERE}/stack/usr/local" ]; then
  echo "1/3 複製 webkit2gtk-4.1 / libsoup3 / glib 2.78 → /usr/local"
  cp -a "${HERE}/stack/usr/local/." /usr/local/
  # 00- 前綴讓它排在 /etc/ld.so.conf.d 其他設定之前，ldconfig 才會優先索引 /usr/local 的新版 .so
  tee /etc/ld.so.conf.d/00-cix3752i-sorter.conf > /dev/null <<'LDCONF'
/usr/local/lib/x86_64-linux-gnu
/usr/local/lib
LDCONF
  ldconfig
else
  echo "1/3 此 distro 走系統 webkit2gtk-4.1，不需自編棧"
fi

# 2. 程式本體
if [ "${MODE}" = "desktop" ]; then
  echo "2/3 安裝 .deb"
  if [ -d "${HERE}/stack/usr/local" ]; then
    # 20.04 離線包：依賴只列系統本來就有的，dpkg 直接裝；裝不上再讓 apt 補
    dpkg -i "${DEB}" || apt-get install -f -y
  else
    apt-get update
    apt-get install -y "${DEB}"
  fi
  # 寫 USB 印表機需要 lp 群組（/dev/usb/lp* 預設 root:lp 660）；桌面模式是登入的那個使用者在跑
  if [ -n "${SUDO_USER:-}" ]; then
    usermod -a -G lp "${SUDO_USER}" || true
  fi
else
  echo "2/3 從 .deb 取出執行檔 → ${APP_DIR}/sorter"
  # headless 不裝 .deb（不要桌面捷徑、也避免與桌面模式的更新路徑混在一起），只取執行檔；
  # 執行期依賴仍照 .deb 的 Depends 補齊（20.04 是 gtk，22.04+ 還有 webkit2gtk-4.1）
  EXTRACT="$(mktemp -d)"
  dpkg-deb -x "${DEB}" "${EXTRACT}"
  if [ ! -d "${HERE}/stack/usr/local" ]; then
    apt-get update
    # shellcheck disable=SC2046
    apt-get install -y --no-install-recommends $(dpkg-deb -f "${DEB}" Depends | tr ',' ' ')
  fi
  mkdir -p "${APP_DIR}/data"
  install -m 755 "${EXTRACT}/usr/bin/sorter" "${APP_DIR}/sorter.new"
  mv -f "${APP_DIR}/sorter.new" "${APP_DIR}/sorter"
  rm -rf "${EXTRACT}"
  chown -R "${APP_USER}:${APP_USER}" "${APP_DIR}"
  usermod -a -G lp "${APP_USER}" || true
fi

# 3. 服務登記
case "${MODE}" in
  desktop)
    echo "3/3 完成。從應用選單開啟「智配通 分揀控制」，或於終端機執行：sorter"
    ;;
  systemd)
    echo "3/3 登記 systemd 服務"
    render "${HERE}/sorter.service" > /etc/systemd/system/cix3752i-sorter.service
    chmod 644 /etc/systemd/system/cix3752i-sorter.service
    systemctl daemon-reload
    systemctl enable cix3752i-sorter
    systemctl restart cix3752i-sorter
    sleep 2
    systemctl --no-pager --lines=5 status cix3752i-sorter || true
    ;;
  supervisor)
    echo "3/3 登記 supervisor 程式"
    # 現場的 supervisord.conf 只 include conf.d/*.ini（廠商裝的），檔名用 .conf 會被無視、程式永遠登記不進去
    render "${HERE}/supervisor-sorter.ini" > /etc/supervisor/conf.d/cix3752i-sorter.ini
    chmod 644 /etc/supervisor/conf.d/cix3752i-sorter.ini
    supervisorctl reread
    supervisorctl update
    supervisorctl restart cix3752i-sorter
    supervisorctl status cix3752i-sorter || true
    ;;
esac

if [ "${MODE}" != "desktop" ]; then
  echo "安裝完成：${APP_DIR}/sorter（$("${APP_DIR}/sorter" --version)）"
fi
