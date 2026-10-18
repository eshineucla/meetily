"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"

export interface GlossaryEntry {
  id: string
  category: string
  canonical: string
  aliases: string[]
  note: string
  enabled: boolean
}

interface GlossaryOverview {
  entries: GlossaryEntry[]
  conflicts: { alias: string; canonicals: string[] }[]
  promptTerms: string[]
  promptTermsTotal: number
}

export interface CorrectionHit {
  alias: string
  canonical: string
  entryId: string
  count: number
}

const CATEGORIES: [string, string][] = [
  ["person", "人名职务"],
  ["org", "机构名"],
  ["codename", "项目代号"],
  ["term", "业务术语"],
]
const CATEGORY_LABEL = Object.fromEntries(CATEGORIES)

const EMPTY: GlossaryEntry = { id: "", category: "person", canonical: "", aliases: [], note: "", enabled: true }

/** Aliases are typed separated by 、，, or | */
function splitAliases(raw: string): string[] {
  return raw.split(/[|、，,;；\n]/).map((a) => a.trim()).filter(Boolean)
}

/** Glossary: correct spellings and their usual misrecognitions, applied after transcription */
export function GlossarySettings() {
  const [overview, setOverview] = useState<GlossaryOverview | null>(null)
  const [editing, setEditing] = useState<GlossaryEntry | null>(null)
  const [aliasText, setAliasText] = useState("")
  const [filter, setFilter] = useState("")
  const [category, setCategory] = useState("")
  const [busy, setBusy] = useState(false)
  const [importProblems, setImportProblems] = useState<string[]>([])
  const [sample, setSample] = useState("")
  const [preview, setPreview] = useState<{ text: string; hits: CorrectionHit[] } | null>(null)

  const load = useCallback(() => {
    invoke<GlossaryOverview>("glossary_list")
      .then(setOverview)
      .catch((error) => toast.error("读取术语库失败", { description: String(error) }))
  }, [])
  useEffect(load, [load])

  const startEdit = (entry: GlossaryEntry) => {
    setEditing({ ...entry })
    setAliasText(entry.aliases.join("、"))
  }

  const save = async () => {
    if (!editing) return
    setBusy(true)
    try {
      await invoke("glossary_save", { entry: { ...editing, aliases: splitAliases(aliasText) } })
      toast.success(editing.id ? "已保存" : "已添加")
      setEditing(null)
      load()
    } catch (error) {
      toast.error("保存失败", { description: String(error) })
    } finally {
      setBusy(false)
    }
  }

  const toggle = async (entry: GlossaryEntry) => {
    try {
      await invoke("glossary_set_enabled", { id: entry.id, enabled: !entry.enabled })
      load()
    } catch (error) {
      toast.error("操作失败", { description: String(error) })
    }
  }

  const remove = async (entry: GlossaryEntry) => {
    if (!window.confirm(`删除「${entry.canonical}」？只是暂时不用的话，可以改成停用。`)) return
    try {
      await invoke("glossary_delete", { id: entry.id })
      load()
    } catch (error) {
      toast.error("删除失败", { description: String(error) })
    }
  }

  const importCsv = async () => {
    setBusy(true)
    try {
      const report = await invoke<{ added: number; updated: number; problems: string[] } | null>("glossary_import_csv")
      if (report) {
        setImportProblems(report.problems)
        const summary = `新增 ${report.added} 条，更新 ${report.updated} 条`
        if (report.problems.length) {
          toast.warning("部分行没有导入", { description: `${summary}，${report.problems.length} 行有问题，见下方列表` })
        } else {
          toast.success("术语表已导入", { description: summary })
        }
        load()
      }
    } catch (error) {
      toast.error("导入失败", { description: String(error) })
    } finally {
      setBusy(false)
    }
  }

  const exportCsv = async () => {
    try {
      const path = await invoke<string | null>("glossary_export_csv")
      if (path) toast.success("术语表已导出", { description: path, duration: 8000 })
    } catch (error) {
      toast.error("导出失败", { description: String(error) })
    }
  }

  const runPreview = async () => {
    try {
      setPreview(await invoke<{ text: string; hits: CorrectionHit[] }>("glossary_preview", { text: sample }))
    } catch (error) {
      toast.error("试用失败", { description: String(error) })
    }
  }

  const shown = useMemo(() => {
    const needle = filter.trim()
    return (overview?.entries ?? []).filter(
      (e) =>
        (!category || e.category === category) &&
        (!needle || e.canonical.includes(needle) || e.aliases.some((a) => a.includes(needle)) || e.note.includes(needle)),
    )
  }, [overview, filter, category])

  return (
    <div className="space-y-6">
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-4">
        <h3 className="text-lg font-semibold text-gray-900">术语库</h3>
        <p className="text-sm text-gray-600">
          记下机构名、人名职务、业务术语和项目代号的正确写法，以及转写常见的错误写法。录音转写、导入音频和导入记录稿时，错误写法会自动改成正确写法；已经写对的词和文件号（如“中办发〔2026〕1号”）不会被改动。每次改了多少处记入审计日志。
        </p>
        {overview && (
          <p className="text-sm text-gray-600">
            使用 Whisper 转写时，还会把正确写法作为提示交给转写引擎：目前放入 {overview.promptTerms.length} / {overview.promptTermsTotal} 个词
            {overview.promptTerms.length < overview.promptTermsTotal ? "（提示长度有限，人名和机构名优先，其余的仍会在转写后自动纠正）" : ""}。
          </p>
        )}
        {overview && overview.conflicts.length > 0 && (
          <div className="p-3 border rounded-md text-sm bg-amber-50 border-amber-200 text-amber-800">
            有错误写法同时指向多个正确写法，程序会选较长的那个，请检查：
            {overview.conflicts.map((c) => `「${c.alias}」→ ${c.canonicals.join(" / ")}`).join("；")}
          </div>
        )}
        <div className="flex flex-wrap items-end gap-3">
          <label className="text-sm text-gray-700 flex flex-col gap-1">
            类别
            <select value={category} onChange={(e) => setCategory(e.target.value)} className="border rounded px-2 py-1">
              <option value="">全部</option>
              {CATEGORIES.map(([value, label]) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
          </label>
          <label className="text-sm text-gray-700 flex flex-col gap-1">
            查找
            <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="正确或错误写法" className="border rounded px-2 py-1" />
          </label>
          <div className="ml-auto flex gap-2">
            <button onClick={() => startEdit(EMPTY)} className="px-3 py-2 text-sm bg-gray-900 text-white rounded-md hover:bg-gray-800">
              添加
            </button>
            <button onClick={importCsv} disabled={busy} className="px-3 py-2 text-sm border rounded-md hover:bg-gray-50 disabled:opacity-50">
              导入 CSV
            </button>
            <button onClick={exportCsv} className="px-3 py-2 text-sm border rounded-md hover:bg-gray-50">
              导出 CSV
            </button>
          </div>
        </div>
        <p className="text-xs text-gray-500">
          CSV 表头为“类别,规范写法,易错写法,备注,启用”，多个易错写法用 | 分隔；也能直接导入旧系统的术语表（category,canonical,aliases,note）。规范写法已存在的行会更新原条目。
        </p>

        {importProblems.length > 0 && (
          <div className="p-3 border rounded-md text-sm bg-amber-50 border-amber-200 text-amber-800 space-y-1">
            <div className="flex justify-between">
              <span>以下行没有导入：</span>
              <button onClick={() => setImportProblems([])} className="text-xs underline">关闭</button>
            </div>
            <ul className="list-disc pl-5 max-h-40 overflow-y-auto">
              {importProblems.map((p, i) => <li key={i}>{p}</li>)}
            </ul>
          </div>
        )}

        {editing && (
          <div className="p-4 border rounded-md bg-gray-50 space-y-3">
            <div className="flex flex-wrap gap-3">
              <label className="text-sm text-gray-700 flex flex-col gap-1">
                类别
                <select
                  value={editing.category}
                  onChange={(e) => setEditing({ ...editing, category: e.target.value })}
                  className="border rounded px-2 py-1"
                >
                  {CATEGORIES.map(([value, label]) => (
                    <option key={value} value={value}>{label}</option>
                  ))}
                </select>
              </label>
              <label className="text-sm text-gray-700 flex flex-col gap-1 flex-1 min-w-[12rem]">
                正确写法
                <input
                  value={editing.canonical}
                  onChange={(e) => setEditing({ ...editing, canonical: e.target.value })}
                  className="border rounded px-2 py-1"
                />
              </label>
            </div>
            <label className="text-sm text-gray-700 flex flex-col gap-1">
              易错写法（用顿号或逗号分开，每个至少两个字）
              <input value={aliasText} onChange={(e) => setAliasText(e.target.value)} className="border rounded px-2 py-1" />
            </label>
            <label className="text-sm text-gray-700 flex flex-col gap-1">
              备注
              <input value={editing.note} onChange={(e) => setEditing({ ...editing, note: e.target.value })} className="border rounded px-2 py-1" />
            </label>
            <div className="flex gap-2 justify-end">
              <button onClick={() => setEditing(null)} className="px-3 py-1.5 text-sm border rounded-md hover:bg-white">取消</button>
              <button
                onClick={save}
                disabled={busy || !editing.canonical.trim()}
                className="px-3 py-1.5 text-sm bg-gray-900 text-white rounded-md hover:bg-gray-800 disabled:opacity-50"
              >
                保存
              </button>
            </div>
          </div>
        )}

        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-500 border-b">
                <th className="py-2 pr-3 font-medium">类别</th>
                <th className="py-2 pr-3 font-medium">正确写法</th>
                <th className="py-2 pr-3 font-medium">易错写法</th>
                <th className="py-2 pr-3 font-medium">备注</th>
                <th className="py-2 font-medium text-right">操作</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((e) => (
                <tr key={e.id} className={`border-b last:border-0 align-top ${e.enabled ? "" : "text-gray-400"}`}>
                  <td className="py-2 pr-3 whitespace-nowrap">{CATEGORY_LABEL[e.category] ?? e.category}</td>
                  <td className="py-2 pr-3 font-medium">{e.canonical}{e.enabled ? "" : "（已停用）"}</td>
                  <td className="py-2 pr-3">{e.aliases.join("、")}</td>
                  <td className="py-2 pr-3 text-gray-500">{e.note}</td>
                  <td className="py-2 whitespace-nowrap text-right space-x-3">
                    <button onClick={() => startEdit(e)} className="text-blue-600 hover:underline">修改</button>
                    <button onClick={() => toggle(e)} className="text-gray-600 hover:underline">{e.enabled ? "停用" : "启用"}</button>
                    <button onClick={() => remove(e)} className="text-red-600 hover:underline">删除</button>
                  </td>
                </tr>
              ))}
              {shown.length === 0 && (
                <tr>
                  <td colSpan={5} className="py-6 text-center text-gray-400">
                    {overview?.entries.length ? "没有符合条件的条目" : "术语库还是空的，可以逐条添加或导入 CSV"}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-3">
        <h3 className="text-base font-semibold text-gray-900">试一试</h3>
        <p className="text-sm text-gray-600">粘贴一段文字，看看术语库会怎么改。只是预览，不会保存。</p>
        <textarea
          value={sample}
          onChange={(e) => setSample(e.target.value)}
          rows={3}
          className="w-full border rounded px-2 py-1 text-sm"
        />
        <button
          onClick={runPreview}
          disabled={!sample.trim()}
          className="px-3 py-1.5 text-sm border rounded-md hover:bg-gray-50 disabled:opacity-50"
        >
          预览纠正结果
        </button>
        {preview && (
          <div className="text-sm space-y-2">
            <div className="p-3 bg-gray-50 border rounded whitespace-pre-wrap">{preview.text}</div>
            <div className="text-gray-600">
              {preview.hits.length
                ? preview.hits.map((h) => `「${h.alias}」→「${h.canonical}」${h.count} 处`).join("；")
                : "没有需要改的地方"}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
