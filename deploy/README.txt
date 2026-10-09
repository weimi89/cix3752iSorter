智配通 分揀控制 — Linux 安裝包
==============================

內含：
  - cix3752i-sorter_<版本>_<distro>_amd64.deb   主程式（桌面視窗 + 內建網頁後台）
  - stack/usr/local/                            只有 Ubuntu 20.04 的包有：自編的 glib 2.78 + libsoup3 + webkit2gtk-4.1
                                                （20.04 套件源沒有 webkit2gtk-4.1，桌面視窗要靠它）
  - install.sh、switch.sh

安裝：
  sudo bash install.sh                只裝不啟動；裝完登出再登入一次（印表機權限）

切換（線上沒包裹時）：
  sorter-switch to-new                停舊程式 → 確認相機與分揀機放掉 → 舊的不再自啟 → 開新程式
  sorter-switch to-old                回退到舊程式（2 分鐘內）
  sorter-switch status                看現在誰在跑

後續升級：
  - 程式內「發現新版本」→ 立即更新（會要系統密碼；或 sudo dpkg -i <新版 .deb>）
  - 發版說明寫「webkit 棧有更新」時（只影響 20.04）：重新解開完整安裝包跑一次 install.sh

不可降版：
  新版開機時若要更新資料庫結構，會先把整個資料庫備份到
    ~/.local/share/com.weiminet.cix3752i.sorter/data/backups/sorter-<時間>-v<舊>-before-v<新>.sqlite
  （只留最近 3 份）。更新後直接裝回舊版的 .deb，舊版會拒絕啟動。真的要降版：
    1. 關掉程式（關視窗，或 pkill -TERM -x sorter）
    2. cd ~/.local/share/com.weiminet.cix3752i.sorter/data
    3. 把現在的資料庫改名留著：mv sorter.sqlite sorter.sqlite.after-upgrade；sorter.sqlite-wal、sorter.sqlite-shm 也一起改名
    4. 把升級前那份複製回來：cp backups/<升級前那份>.sqlite sorter.sqlite
    5. sudo dpkg -i <舊版 .deb>，再開程式
  升級之後到降版之間的包裹紀錄不會出現在舊版裡（還在改名留著的那份）。
  sorter-switch to-old 回退的是舊系統，用的是另一套資料，不受影響。
