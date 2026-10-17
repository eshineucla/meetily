"use client"

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { FileText } from "lucide-react"
import { toast } from "sonner"

/** Choose the Word template the minutes are exported into, or keep the built-in layout */
export function MinutesTemplateSettings() {
  const [templateName, setTemplateName] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    invoke<string | null>("get_minutes_template_command")
      .then(setTemplateName)
      .catch((error) => console.error("Failed to load Word template:", error))
  }, [])

  const chooseTemplate = async () => {
    setBusy(true)
    try {
      const result = await invoke<{ name: string; warnings: string[] } | null>("choose_minutes_template_command")
      if (!result) return
      setTemplateName(result.name)
      if (result.warnings.length > 0) {
        toast.warning("已使用该模板，但有几处需要注意", { description: result.warnings.join("\n"), duration: 10000 })
      } else {
        toast.success("已使用该模板导出 Word")
      }
    } catch (error) {
      toast.error("无法使用该模板", { description: String(error) })
    } finally {
      setBusy(false)
    }
  }

  const clearTemplate = async () => {
    setBusy(true)
    try {
      await invoke("clear_minutes_template_command")
      setTemplateName(null)
      toast.success("已恢复内置公文版式")
    } catch (error) {
      toast.error("恢复失败", { description: String(error) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
      <h3 className="text-lg font-semibold text-gray-900 mb-2">Word 导出模板</h3>
      <p className="text-sm text-gray-600 mb-4">
        默认按内置的公文版式导出。也可以选择本单位的 Word 模板（.dotx / .docx），导出时套用模板里的“标题”“标题 1–3”“正文”样式、页面设置和页码，模板原有的示例文字不会保留。
      </p>
      <div className="p-4 border rounded-lg bg-gray-50 flex items-center justify-between gap-4">
        <div className="flex items-center gap-2 min-w-0">
          <FileText className="w-4 h-4 shrink-0 text-gray-500" />
          <span className="text-sm text-gray-800 truncate">{templateName ?? "内置公文版式"}</span>
        </div>
        <div className="flex gap-2 shrink-0">
          <button
            onClick={chooseTemplate}
            disabled={busy}
            className="px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:opacity-50"
          >
            {templateName ? "更换模板" : "选择模板"}
          </button>
          {templateName && (
            <button
              onClick={clearTemplate}
              disabled={busy}
              className="px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:opacity-50"
            >
              恢复内置版式
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
