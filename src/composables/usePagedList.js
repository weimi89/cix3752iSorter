/**
 * 分頁清單頁共用的載入流程：頁碼、每頁筆數、總數、清單、載入中、錯誤訊息。
 *
 * - 只採用最後一次查詢的結果：自動更新的第 1 頁還沒回來時翻到第 2 頁，晚到的第 1 頁不能蓋掉第 2 頁
 * - 搜尋與改每頁筆數會回到第 1 頁；頁碼本來就是 1 才自己查，否則交給頁碼變化去查，不重複送
 * - `refreshSoon` 給即時事件用（短時間內多次只查一次），離開頁面時計時器一併清掉
 *
 * `fetchPage({ page, limit })` 回 `{ total, list }`，篩選條件由呼叫端自己帶進去。
 */
import { useLatestRequest } from '@/composables/useLatestRequest'

export function usePagedList(fetchPage, { pageSize: initialSize = 25 } = {}) {
  const page = ref(1)
  const pageSize = ref(initialSize)
  const total = ref(0)
  const list = ref([])
  const loading = ref(false)
  const errorMsg = ref('')
  const latest = useLatestRequest()

  const load = async () => {
    const id = latest.start()
    loading.value = true
    errorMsg.value = ''
    try {
      const d = await fetchPage({ page: page.value, limit: pageSize.value })
      if (!latest.isLatest(id)) return
      total.value = d.total
      list.value = d.list
    } catch (e) {
      if (latest.isLatest(id)) errorMsg.value = e.message
    } finally {
      if (latest.isLatest(id)) loading.value = false
    }
  }
  const search = () => { if (page.value !== 1) page.value = 1; else load() }

  let timer = null
  const refreshSoon = ms => { clearTimeout(timer); timer = setTimeout(load, ms) }

  watch(pageSize, search)
  watch(page, load)
  onBeforeUnmount(() => clearTimeout(timer))

  return { page, pageSize, total, list, loading, errorMsg, load, search, refreshSoon }
}
