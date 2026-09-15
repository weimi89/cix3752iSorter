#!/usr/bin/env node
/**
 * 從 @iconify-json 抽出「專案實際用到的那些圖示」，產生離線圖示集。
 *
 * 為什麼要這個:`@iconify/vue` 的 `Icon` 在本地找不到圖示資料時，會去 api.iconify.design
 * 線上抓。中介機為了打雲端 API 本來就有外網，所以現場一直沒出事 —— 但只要外網一斷
 * (工廠網路不穩、防火牆改設定)，**畫面上每一個圖示都會變成空白**，導覽列的按鈕全部
 * 看不出是什麼。這個 App 本身有三層網路偵測就是為了撐過斷網，UI 卻在斷網時先瞎掉。
 *
 * 為什麼是「子集」而不是整包:tabler 有數千個圖示，整包打進去會讓 bundle 多好幾 MB，
 * 而這次才剛把首次載入壓下來。只收實際用到的一百多個，代價只有幾十 KB。
 *
 * 用法:`node scripts/build-icon-subset.mjs`
 * 什麼時候要重跑:**新增或改用了新的圖示之後**。
 * 忘了重跑會被 `tests/maintenance-guards.test.mjs` 擋下來，不會默默漏掉。
 */

import { readFileSync, writeFileSync, readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = resolve(fileURLToPath(new URL('.', import.meta.url)), '..')
const OUT = join(root, 'src/plugins/icons-offline.json')

/** 掃描原始碼，找出所有以字面值寫死的圖示名（tabler-xxx / mdi-xxx） */
export function collectUsedIcons(srcDir = join(root, 'src')) {
  const found = new Set()
  const walk = dir => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name)
      if (e.isDirectory()) { walk(p); continue }
      if (!/\.(vue|js|ts)$/.test(e.name)) continue
      const text = readFileSync(p, 'utf8')
      for (const m of text.matchAll(/["'`](tabler-[a-z0-9-]+|mdi-[a-z0-9-]+)["'`]/g)) {
        found.add(m[1])
      }
    }
  }
  walk(srcDir)
  return found
}

/** `tabler-device-tv` → `['tabler', 'device-tv']` */
const split = name => {
  const i = name.indexOf('-')
  return [name.slice(0, i), name.slice(i + 1)]
}

function build() {
  const used = collectUsedIcons()
  const byPrefix = {}
  for (const full of used) {
    const [prefix, icon] = split(full)
    ;(byPrefix[prefix] ||= new Set()).add(icon)
  }

  const out = {}
  const missing = []

  for (const [prefix, wanted] of Object.entries(byPrefix)) {
    const srcPath = join(root, `node_modules/@iconify-json/${prefix}/icons.json`)
    let full
    try {
      full = JSON.parse(readFileSync(srcPath, 'utf8'))
    } catch {
      missing.push(`${prefix}（找不到 @iconify-json/${prefix}）`)
      continue
    }

    const icons = {}
    for (const name of wanted) {
      // 別名要順著指到本體，否則抽出來會是空的
      const resolved = full.aliases?.[name]?.parent ?? name
      const body = full.icons?.[resolved]
      if (!body) { missing.push(`${prefix}:${name}`); continue }
      icons[name] = full.aliases?.[name]
        ? { ...body, ...full.aliases[name], parent: undefined }
        : body
    }

    out[prefix] = {
      prefix,
      icons,
      width: full.width,
      height: full.height,
      ...(full.lastModified ? { lastModified: full.lastModified } : {}),
    }
  }

  writeFileSync(OUT, JSON.stringify(out), 'utf8')

  const total = Object.values(out).reduce((n, c) => n + Object.keys(c.icons).length, 0)
  const kb = (readFileSync(OUT).length / 1024).toFixed(0)
  console.log(`已產生 ${OUT.replace(root + '/', '')}`)
  for (const [p, c] of Object.entries(out)) console.log(`  ${p}: ${Object.keys(c.icons).length} 個`)
  console.log(`  合計 ${total} 個，${kb} KB`)
  if (missing.length) {
    console.log(`\n⚠ 這些在圖示集裡找不到（畫面上會是空白，請確認名稱）：`)
    missing.forEach(m => console.log(`  ${m}`))
    process.exitCode = 1
  }
}

// 用 pathToFileURL 比對:專案路徑含中文字，直接串 `file://` 會因為百分比編碼對不上而永遠為 false
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) build()
