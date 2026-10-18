"use client"

import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"

interface AuditEntry {
  log_id: number
  ts: string
  operator: string
  action: string
  label: string
  target_type: string | null
  target_id: string | null
  detail: string
  result: string
}

interface AuditStatus {
  total_rows: number
  integrity_ok: boolean
  problems: { log_id: number; kind: string; explanation: string }[]
  check_items: Record<string, "ok" | "empty">
  actions: [string, string][]
  disk: { state: "on" | "off" | "in_progress" | "unknown"; drive: string; message: string }
}

const PAGE_SIZE = 50

/** Local date (yyyy-mm-dd) to the UTC second the log uses */
function toUtc(date: string, endOfDay: boolean): string | null {
  if (!date) return null
  const local = new Date(`${date}T${endOfDay ? "23:59:59" : "00:00:00"}`)
  return local.toISOString().replace(/\.\d{3}Z$/, "Z")
}

function formatDetail(detail: string): string {
  if (!detail || detail === "{}") return ""
  try {
    return Object.entries(JSON.parse(detail) as Record<string, unknown>)
      .map(([k, v]) => `${k}: ${typeof v === "object" ? JSON.stringify(v) : String(v)}`)
      .join("，")
  } catch {
    return detail
  }
}

const DISK_STYLE: Record<AuditStatus["disk"]["state"], string> = {
  on: "bg-green-50 border-green-200 text-green-800",
  in_progress: "bg-amber-50 border-amber-200 text-amber-800",
  off: "bg-red-50 border-red-200 text-red-800",
  unknown: "bg-amber-50 border-amber-200 text-amber-800",
}

/** Audit trail: integrity, disk encryption, check items, the log itself and the report export */
export function AuditLogSettings() {
  const [status, setStatus] = useState<AuditStatus | null>(null)
  const [entries, setEntries] = useState<AuditEntry[]>([])
  const [total, setTotal] = useState(0)
  const [page, setPage] = useState(0)
  const [from, setFrom] = useState("")
  const [to, setTo] = useState("")
  const [action, setAction] = useState("")
  const [exporting, setExporting] = useState(false)

  const loadStatus = useCallback(() => {
    invoke<AuditStatus>("audit_status")
      .then(setStatus)
      .catch((error) => toast.error("读取审计状态失败", { description: String(error) }))
  }, [])

  const loadPage = useCallback(() => {
    invoke<{ entries: AuditEntry[]; total: number }>("audit_list", {
      from: toUtc(from, false),
      to: toUtc(to, true),
      action: action || null,
      offset: page * PAGE_SIZE,
      limit: PAGE_SIZE,
    })
      .then((result) => {
        setEntries(result.entries)
        setTotal(result.total)
      })
      .catch((error) => toast.error("读取审计日志失败", { description: String(error) }))
  }, [from, to, action, page])

  useEffect(loadStatus, [loadStatus])
  useEffect(loadPage, [loadPage])

  const exportReport = async () => {
    setExporting(true)
    try {
      const path = await invoke<string | null>("audit_export_report", { from: toUtc(from, false), to: toUtc(to, true) })
      if (path) {
        toast.success("审计报告已导出", { description: path, duration: 8000 })
        loadStatus()
        loadPage()
      }
    } catch (error) {
      toast.error("导出审计报告失败", { description: String(error) })
    } finally {
      setExporting(false)
    }
  }

  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE))

  return (
    <div className="space-y-6">
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-4">
        <h3 className="text-lg font-semibold text-gray-900">审计留痕</h3>
        <p className="text-sm text-gray-600">
          程序记录谁在什么时间对哪场会议做了什么（录音、导入、生成纪要、修改、定稿、导出、删除、配置变更），只记编号和数量，不记会议内容。日志只能追加，任何修改或删除都会让完整性校验失败。
        </p>
        {status && (
          <>
            <div
              className={`p-3 border rounded-md text-sm ${
                status.integrity_ok ? "bg-green-50 border-green-200 text-green-800" : "bg-red-50 border-red-200 text-red-800"
              }`}
            >
              {status.integrity_ok
                ? `完整性校验通过，共 ${status.total_rows} 条记录`
                : `完整性校验未通过：${status.problems
                    .map((p) => `第 ${p.log_id} 条${p.explanation}`)
                    .join("；")}。请立即停止使用，封存设备并上报保密机构，不要自行修复。`}
            </div>
            <div className={`p-3 border rounded-md text-sm ${DISK_STYLE[status.disk.state]}`}>
              磁盘加密：{status.disk.message}
            </div>
            <div className="flex flex-wrap gap-2">
              {Object.entries(status.check_items).map(([item, state]) => (
                <span
                  key={item}
                  className={`px-2 py-1 rounded text-xs border ${
                    state === "ok" ? "bg-gray-50 border-gray-200 text-gray-700" : "bg-white border-dashed border-gray-300 text-gray-400"
                  }`}
                  title={state === "ok" ? "已有记录" : "还没有记录"}
                >
                  {item}
                  {state === "ok" ? " ✓" : ""}
                </span>
              ))}
            </div>
          </>
        )}
      </div>

      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-4">
        <div className="flex flex-wrap items-end gap-3">
          <label className="text-sm text-gray-700 flex flex-col gap-1">
            起始日期
            <input type="date" value={from} onChange={(e) => { setFrom(e.target.value); setPage(0) }} className="border rounded px-2 py-1" />
          </label>
          <label className="text-sm text-gray-700 flex flex-col gap-1">
            截止日期
            <input type="date" value={to} onChange={(e) => { setTo(e.target.value); setPage(0) }} className="border rounded px-2 py-1" />
          </label>
          <label className="text-sm text-gray-700 flex flex-col gap-1">
            动作
            <select value={action} onChange={(e) => { setAction(e.target.value); setPage(0) }} className="border rounded px-2 py-1">
              <option value="">全部</option>
              {status?.actions.map(([value, label]) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
          </label>
          <button
            onClick={exportReport}
            disabled={exporting}
            className="ml-auto px-3 py-2 text-sm bg-gray-900 text-white rounded-md hover:bg-gray-800 transition-colors disabled:opacity-50"
          >
            {exporting ? "正在导出…" : "导出审计报告"}
          </button>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-500 border-b">
                <th className="py-2 pr-3 font-medium">时间</th>
                <th className="py-2 pr-3 font-medium">操作人</th>
                <th className="py-2 pr-3 font-medium">动作</th>
                <th className="py-2 pr-3 font-medium">对象</th>
                <th className="py-2 pr-3 font-medium">详情</th>
                <th className="py-2 font-medium">结果</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => (
                <tr key={e.log_id} className="border-b last:border-0 align-top">
                  <td className="py-2 pr-3 whitespace-nowrap text-gray-700">{new Date(e.ts).toLocaleString("zh-CN")}</td>
                  <td className="py-2 pr-3 text-gray-700">{e.operator}</td>
                  <td className="py-2 pr-3 text-gray-900">{e.label}</td>
                  <td className="py-2 pr-3 text-gray-500 font-mono text-xs break-all">{e.target_id ?? ""}</td>
                  <td className="py-2 pr-3 text-gray-500 text-xs break-all">{formatDetail(e.detail)}</td>
                  <td className={`py-2 ${e.result === "success" ? "text-gray-700" : "text-red-600"}`}>
                    {e.result === "success" ? "成功" : "失败"}
                  </td>
                </tr>
              ))}
              {entries.length === 0 && (
                <tr>
                  <td colSpan={6} className="py-6 text-center text-gray-400">没有记录</td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        {total > PAGE_SIZE && (
          <div className="flex items-center justify-end gap-3 text-sm text-gray-600">
            <button disabled={page === 0} onClick={() => setPage(page - 1)} className="px-2 py-1 border rounded disabled:opacity-40">上一页</button>
            <span>第 {page + 1} / {pages} 页，共 {total} 条</span>
            <button disabled={page + 1 >= pages} onClick={() => setPage(page + 1)} className="px-2 py-1 border rounded disabled:opacity-40">下一页</button>
          </div>
        )}
      </div>
    </div>
  )
}
