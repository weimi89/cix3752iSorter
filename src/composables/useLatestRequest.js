/**
 * 同一份資料連續查好幾次時（改日期、切篩選、自動更新），只採用最後一次送出的結果。
 * 回應回來的順序不一定跟送出順序相同：晚到的舊回應若照樣寫進畫面，條件已經換了、內容卻是舊的。
 *
 *     const latest = useLatestRequest()
 *     const id = latest.start()
 *     const r = await api.x()
 *     if (!latest.isLatest(id)) return
 */
export function useLatestRequest() {
  let seq = 0
  return {
    start: () => ++seq,
    isLatest: id => id === seq,
  }
}
