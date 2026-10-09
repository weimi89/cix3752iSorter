/**
 * 掛載時訂閱後端即時事件、卸載時取消。
 *
 * 訂閱一定要在掛載當下同步建立：寫在 `await` 之後的話，等待期間離開頁面，卸載時還沒有可取消的訂閱，
 * 回應一到才訂閱，之後每個事件都替已經關掉的頁面查一次、而且永遠不會取消。
 * 同一頁要訂閱幾個事件就呼叫幾次。
 */
import { listen } from '@/api/events'

export function useServerEvent(name, cb) {
  let off = null
  onMounted(() => { off = listen(name, cb) })
  onBeforeUnmount(() => { off?.(); off = null })
}
