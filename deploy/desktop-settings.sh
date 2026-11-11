#!/bin/bash
# Ubuntu 20.04 工控機：讓桌面讀得到使用者設定（螢幕不關、不鎖定、不休眠）。
#
#   sudo bash desktop-settings.sh
#
# install.sh 會自動呼叫；已經裝好的機台單獨跑一次，跑完登出再登入（或重開機）才生效。
# 沒跑會怎樣：install.sh 讓整個桌面改載 /usr/local 的自編 glib 2.78，它只到 /usr/local 的
# gio 模組目錄找 dconf 模組，找不到就退回「只用預設值」——閒置 5 分鐘關螢幕並鎖定，
# 設定畫面或 gsettings 改了也不會生效（gsettings 讀到的也是預設值，看不出問題，要用 dconf read 對照）。
# 重複執行無害。
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "要用 sudo 執行：sudo bash desktop-settings.sh"; exit 1; }

LOCAL_MODULES=/usr/local/lib/x86_64-linux-gnu/gio/modules
SYSTEM_DCONF=/usr/lib/x86_64-linux-gnu/gio/modules/libdconfsettings.so

if [ ! -e /usr/local/lib/x86_64-linux-gnu/libgio-2.0.so.0 ]; then
  echo "這台沒有自編 glib（/usr/local），桌面走系統 glib，不需要處理"
  exit 0
fi
[ -f "${SYSTEM_DCONF}" ] || {
  echo "系統沒有 dconf 模組 ${SYSTEM_DCONF}：先 sudo apt-get install dconf-gsettings-backend 再跑一次"
  exit 1
}

mkdir -p "${LOCAL_MODULES}"
# 系統的 dconf 模組可以給 glib 2.78 用（10-30 在 cat-taoyuan 以 GIO_EXTRA_MODULES 實測讀得回使用者設定）
[ -e "${LOCAL_MODULES}/libdconfsettings.so" ] || ln -s "${SYSTEM_DCONF}" "${LOCAL_MODULES}/libdconfsettings.so"

# 目錄裡有 giomodule.cache 時，GIO 只載入快取裡列的模組：放進新模組後一定要重建，否則照樣讀不到
QUERYMODULES=/usr/local/bin/gio-querymodules
[ -x "${QUERYMODULES}" ] || QUERYMODULES=/usr/lib/x86_64-linux-gnu/glib-2.0/gio-querymodules
"${QUERYMODULES}" "${LOCAL_MODULES}"
grep -q 'libdconfsettings.so.*gsettings-backend' "${LOCAL_MODULES}/giomodule.cache" || {
  echo "重建後的模組快取裡沒有 dconf（${LOCAL_MODULES}/giomodule.cache），請回報"
  exit 1
}

# 對照：以登入使用者身分讀同一個設定，自編 glib（gsettings）應與 dconf 的存值一致
if [ -n "${SUDO_USER:-}" ]; then
  for key in "org.gnome.desktop.session idle-delay:/org/gnome/desktop/session/idle-delay" \
             "org.gnome.desktop.screensaver lock-enabled:/org/gnome/desktop/screensaver/lock-enabled"; do
    schema_key="${key%%:*}"; path="${key#*:}"
    # shellcheck disable=SC2086  # schema 與鍵名要拆成兩個參數
    via_glib="$(sudo -H -u "${SUDO_USER}" gsettings get ${schema_key} 2>/dev/null || echo '讀不到')"
    stored="$(sudo -H -u "${SUDO_USER}" dconf read "${path}" 2>/dev/null || true)"
    echo "  ${schema_key}：桌面讀到 ${via_glib}；dconf 存值 ${stored:-（沒設，用預設）}"
  done
fi

echo "完成：dconf 模組已接上自編 glib。請登出再登入（或重開機）讓桌面程序重新載入。"
