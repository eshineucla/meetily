"use client"

import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Package } from "lucide-react"
import { toast } from "sonner"
import { CLASSIFIED_BUILD } from "@/config/features"
import {
  formatPackSize,
  importModelPack,
  ModelPackCandidate,
  useModelPackProgress,
} from "@/lib/model-pack"

/** Install models from a model pack, or write the models of this machine into one */
export function ModelPackSettings() {
  const [busy, setBusy] = useState<"import" | "export" | null>(null)
  const [candidates, setCandidates] = useState<ModelPackCandidate[] | null>(null)
  const [selected, setSelected] = useState<string[]>([])
  const progress = useModelPackProgress()

  const runImport = async () => {
    setBusy("import")
    try {
      await importModelPack()
    } catch {
      // importModelPack already showed the error
    } finally {
      setBusy(null)
    }
  }

  const openExport = async () => {
    try {
      const list = await invoke<ModelPackCandidate[]>("model_pack_list_installed")
      setCandidates(list)
      setSelected(list.filter((c) => c.kind !== "whisper").map((c) => c.id))
    } catch (error) {
      toast.error("读取模型列表失败", { description: String(error) })
    }
  }

  const runExport = async () => {
    setBusy("export")
    try {
      const path = await invoke<string | null>("model_pack_export", { modelIds: selected })
      if (path) {
        toast.success("模型包已保存", { description: path, duration: 8000 })
        setCandidates(null)
      }
    } catch (error) {
      toast.error("导出模型包失败", { description: String(error) })
    } finally {
      setBusy(null)
    }
  }

  const toggle = (id: string) =>
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]))

  const selectedBytes = (candidates ?? []).filter((c) => selected.includes(c.id)).reduce((sum, c) => sum + c.bytes, 0)
  const showProgress = busy !== null && progress !== null && progress.phase === busy && progress.totalBytes > 0

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
      <h3 className="text-lg font-semibold text-gray-900 mb-2">模型包</h3>
      <p className="text-sm text-gray-600 mb-4">
        {CLASSIFIED_BUILD
          ? "涉密版不联网下载模型。请在联网电脑上用标准版下载模型后导出模型包，拷贝到本机导入。"
          : "无法联网下载模型的电脑（如涉密版）可以导入模型包。在本机下载好模型后，可在这里导出模型包，拷贝到其他电脑导入。"}
      </p>
      <div className="flex gap-2">
        <button
          onClick={runImport}
          disabled={busy !== null}
          className="px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:opacity-50 flex items-center gap-2"
        >
          <Package className="w-4 h-4" />
          {busy === "import" ? "正在导入…" : "导入模型包"}
        </button>
        {!CLASSIFIED_BUILD && (
          <button
            onClick={openExport}
            disabled={busy !== null}
            className="px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:opacity-50"
          >
            导出模型包
          </button>
        )}
      </div>

      {showProgress && (
        <div className="mt-4 space-y-1">
          <div className="w-full h-2 bg-gray-200 rounded-full overflow-hidden">
            <div
              className="h-full bg-gray-800 rounded-full transition-all duration-300"
              style={{ width: `${Math.min(100, (progress.doneBytes / progress.totalBytes) * 100)}%` }}
            />
          </div>
          <p className="text-xs text-gray-500">
            {progress.current} · {formatPackSize(progress.doneBytes)} / {formatPackSize(progress.totalBytes)}
          </p>
        </div>
      )}

      {candidates && (
        <div className="mt-4 p-4 border rounded-lg bg-gray-50 space-y-2">
          {candidates.length === 0 ? (
            <p className="text-sm text-gray-600">本机还没有下载完整的模型。</p>
          ) : (
            <>
              <p className="text-sm text-gray-700">选择要放进模型包的模型：</p>
              {candidates.map((c) => (
                <label key={c.id} className="flex items-center gap-2 text-sm text-gray-800">
                  <input type="checkbox" checked={selected.includes(c.id)} onChange={() => toggle(c.id)} />
                  <span className="flex-1">{c.label}</span>
                  <span className="text-gray-500">{formatPackSize(c.bytes)}</span>
                </label>
              ))}
            </>
          )}
          <div className="flex gap-2 pt-2">
            {candidates.length > 0 && (
              <button
                onClick={runExport}
                disabled={busy !== null || selected.length === 0}
                className="px-3 py-2 text-sm bg-gray-900 text-white rounded-md hover:bg-gray-800 transition-colors disabled:opacity-50"
              >
                {busy === "export" ? "正在导出…" : `导出（${formatPackSize(selectedBytes)}）`}
              </button>
            )}
            <button
              onClick={() => setCandidates(null)}
              disabled={busy !== null}
              className="px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:opacity-50"
            >
              取消
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
