"use client";

import { CheckCircle2, AlertTriangle, X, Loader2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ISSUE_LABELS, MinutesCheckResult } from '@/lib/minutes-check';

interface MinutesCheckPanelProps {
  result: MinutesCheckResult | null;
  isChecking: boolean;
  onClose: () => void;
  onRecheck: () => void;
  /** Set when the check ran because the user asked to finalize */
  onFinalizeAnyway?: () => void;
}

export function MinutesCheckPanel({ result, isChecking, onClose, onRecheck, onFinalizeAnyway }: MinutesCheckPanelProps) {
  return (
    <aside className="w-80 flex-shrink-0 border-l border-gray-200 bg-gray-50 flex flex-col min-h-0">
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200">
        <h3 className="font-medium text-gray-900">定稿前检查</h3>
        <button onClick={onClose} className="p-1 rounded hover:bg-gray-200" aria-label="关闭检查">
          <X className="w-4 h-4" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-3 text-sm">
        {isChecking || !result ? (
          <div className="flex items-center gap-2 text-gray-500">
            <Loader2 className="w-4 h-4 animate-spin" /> 正在检查…
          </div>
        ) : result.items.length === 0 ? (
          <p className="text-gray-600">
            没有找到“会议基本情况”“议定事项”“分工落实”之类的章节，无法检查。请用“会议纪要（公文格式）”或党组会、专题会、协调会、例会纪要模板生成纪要。
          </p>
        ) : (
          <>
            <p className={result.issueCount > 0 ? 'text-amber-700' : 'text-green-700'}>
              共检查 {result.items.length} 项，{result.issueCount > 0 ? `${result.issueCount} 项有问题` : '都没有问题'}。
            </p>
            <p className="text-xs text-gray-500">
              规则：会议时间、地点、主持人、参会人员、议题要填写；任务要有责任人和完成时限；每条事项要注明原文出处，且出处能在记录稿中找到；不用口语或含糊表述。
            </p>
            {result.items.map((item, index) => (
              <div
                key={index}
                className={`rounded-md border p-2 ${item.issues.length > 0 ? 'border-amber-300 bg-amber-50' : 'border-gray-200 bg-white'}`}
              >
                <div className="flex items-start gap-1.5">
                  {item.issues.length > 0 ? (
                    <AlertTriangle className="w-4 h-4 text-amber-600 flex-shrink-0 mt-0.5" />
                  ) : (
                    <CheckCircle2 className="w-4 h-4 text-green-600 flex-shrink-0 mt-0.5" />
                  )}
                  <div className="min-w-0">
                    <div className="text-xs text-gray-500">{item.section}</div>
                    <div className="text-gray-900 break-words">{item.text}</div>
                    {item.issues.length > 0 && (
                      <div className="mt-1 flex flex-wrap gap-1">
                        {item.issues.map((issue) => (
                          <span key={issue} className="text-xs px-1.5 py-0.5 rounded bg-amber-200 text-amber-900">
                            {ISSUE_LABELS[issue]}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              </div>
            ))}
          </>
        )}
      </div>

      <div className="p-3 border-t border-gray-200 flex gap-2">
        <Button variant="outline" size="sm" onClick={onRecheck} disabled={isChecking}>
          重新检查
        </Button>
        {onFinalizeAnyway && result && !isChecking && (
          <Button size="sm" onClick={onFinalizeAnyway}>
            {result.issueCount > 0 ? '仍然定稿' : '定稿'}
          </Button>
        )}
      </div>
    </aside>
  );
}
