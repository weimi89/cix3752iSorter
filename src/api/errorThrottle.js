/**
 * 錯誤回報的節流：同一則訊息 `dedupMs` 內只送一次、每分鐘最多 `maxPerMinute` 則。
 *
 * 去重用的表最多留 `maxKeys` 筆：桌面版一開就是好幾天，訊息帶著包裹編號這類每次不同的內容時，
 * 不設上限的表會一直長大、記憶體跟著漲。超過上限先丟掉已過去重期的，還是太多就從最舊的丟——
 * 被丟掉的訊息頂多早一點再回報一次，仍受每分鐘上限管著。
 */
export function createThrottle({ dedupMs = 30_000, maxPerMinute = 20, maxKeys = 200 } = {}) {
  // Map 依放入順序走訪：重新放入前先刪掉，最舊的永遠在最前面
  const recent = new Map()
  let windowStart = 0
  let windowCount = 0

  const prune = now => {
    for (const [k, t] of recent) {
      if (recent.size <= maxKeys) break
      if (t > now - dedupMs) break
      recent.delete(k)
    }
    for (const k of recent.keys()) {
      if (recent.size <= maxKeys) break
      recent.delete(k)
    }
  }

  return {
    allow(key, now) {
      if ((recent.get(key) || 0) > now - dedupMs) return false
      recent.delete(key)
      recent.set(key, now)
      if (recent.size > maxKeys) prune(now)
      if (now - windowStart > 60_000) { windowStart = now; windowCount = 0 }
      return ++windowCount <= maxPerMinute
    },
    size: () => recent.size,
  }
}
