#!/bin/bash
# 工控機安裝／升級（解開發版的 cix3752iSorter-<ver>-<distro>.tar.gz 後於該目錄執行）：
#
#   sudo bash install.sh
#
# 裝 .deb（桌面程式，從應用選單開「智配通 分揀控制」，登入後會自動啟動）與切換工具 sorter-switch。
# 只裝、不啟動：新舊程式搶同一台相機與分揀機，什麼時候換要用 sorter-switch to-new 自己挑時間做。
# 之後升級由程式內建的「發現新版本」處理（會要系統密碼）。
#
# 安裝包若含 stack/（Ubuntu 20.04 版），代表這個 distro 沒有 webkit2gtk-4.1，會先把自編的
# glib／libsoup3／webkit 放進 /usr/local；日後只升程式（程式內更新）不必重跑這段，
# 但發版說明寫「webkit 棧有更新」時就要重跑完整安裝包。
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"

DEB="$(find "${HERE}" -maxdepth 1 -name 'cix3752i-sorter_*.deb' -print -quit)"
[ -n "${DEB}" ] || { echo "找不到 cix3752i-sorter_*.deb，請在解開的安裝包目錄執行"; exit 1; }
[ "$(id -u)" -eq 0 ] || { echo "要用 sudo 執行：sudo bash install.sh"; exit 1; }

echo "=== 智配通 分揀控制 — 安裝 ==="

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
echo "2/3 安裝 .deb"
if [ -d "${HERE}/stack/usr/local" ]; then
  # 20.04 離線包：依賴只列系統本來就有的，dpkg 直接裝；裝不上再讓 apt 補
  dpkg -i "${DEB}" || apt-get install -f -y
else
  apt-get update
  apt-get install -y "${DEB}"
fi
# 寫 USB 印表機需要 lp 群組（/dev/usb/lp* 預設 root:lp 660）；程式是登入的那個使用者在跑。
# 群組要重新登入才生效——裝完記得登出再登入一次，不然切換當天面單會印不出來
if [ -n "${SUDO_USER:-}" ]; then
  usermod -a -G lp "${SUDO_USER}" || true
fi

# 3. 切換／回退工具
echo "3/3 安裝 sorter-switch"
install -m 755 "${HERE}/switch.sh" /usr/local/bin/sorter-switch

echo "完成：$(sorter --version)。"
echo "  ・請登出再登入一次（印表機權限才會生效）"
echo "  ・要換到新程式時：sorter-switch to-new；回退：sorter-switch to-old；看狀態：sorter-switch status"
