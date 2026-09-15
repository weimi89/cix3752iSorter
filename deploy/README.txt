智配通 分揀控制 — Linux 安裝包
==============================

內含：
  - cix3752i-sorter_<版本>_<distro>_amd64.deb   主程式（桌面視窗 + 內建網頁後台；同一個檔也能 --headless 跑）
  - stack/usr/local/                            只有 Ubuntu 20.04 的包有：自編的 glib 2.78 + libsoup3 + webkit2gtk-4.1
                                                （20.04 套件源沒有 webkit2gtk-4.1，桌面視窗與執行檔都要靠它）
  - install.sh、sorter.service、supervisor-sorter.ini

安裝（一台機器只選一種）：
  sudo bash install.sh desktop      桌面模式：從應用選單開「智配通 分揀控制」；新版由程式內建更新安裝（會要系統密碼）
  sudo bash install.sh systemd      無畫面：systemd 以 --headless 拉起，瀏覽器開 http://<ip>:18090；新版由網頁後台換檔
  sudo bash install.sh supervisor   同上，改用 supervisor

20.04 的包可離線安裝；22.04 / 24.04 的包要能連 apt 套件源解依賴（webkit2gtk-4.1 等）。

後續升級：
  - 桌面模式：程式內「發現新版本」→ 立即更新（或 sudo dpkg -i <新版 .deb>）
  - headless：網頁後台「立即更新」（或重跑 sudo bash install.sh systemd|supervisor）
  - 發版說明寫「webkit 棧有更新」時（只影響 20.04）：重新解開完整安裝包跑一次 install.sh
