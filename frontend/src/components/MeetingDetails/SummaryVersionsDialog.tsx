"use client";

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';

export interface SummaryVersionInfo {
  version: number;
  created_at: string;
  source: 'generated' | 'edited' | string;
  markdown: string;
}

interface SummaryVersionsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  /** Restoring is not allowed once the minutes are final */
  readOnly: boolean;
  /** Called after a version was made the current summary */
  onRestored: () => Promise<void> | void;
}

type DiffLine = { kind: 'same' | 'added' | 'removed'; text: string };

/** Line diff of `after` against `before` (longest common subsequence) */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split('\n');
  const b = after.split('\n');
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () => new Array(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const lines: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      lines.push({ kind: 'same', text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      lines.push({ kind: 'removed', text: a[i++] });
    } else {
      lines.push({ kind: 'added', text: b[j++] });
    }
  }
  while (i < a.length) lines.push({ kind: 'removed', text: a[i++] });
  while (j < b.length) lines.push({ kind: 'added', text: b[j++] });
  return lines;
}

const formatTime = (iso: string) => {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString('zh-CN', { hour12: false });
};

export function SummaryVersionsDialog({ open, onOpenChange, meetingId, readOnly, onRestored }: SummaryVersionsDialogProps) {
  const [versions, setVersions] = useState<SummaryVersionInfo[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [compareWith, setCompareWith] = useState<number | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isRestoring, setIsRestoring] = useState(false);

  useEffect(() => {
    if (!open) return;
    setIsLoading(true);
    invoke<SummaryVersionInfo[]>('api_list_summary_versions', { meetingId })
      .then((list) => {
        setVersions(list);
        setSelected(list[0]?.version ?? null);
        setCompareWith(null);
      })
      .catch((error) => toast.error('读取纪要版本失败', { description: String(error) }))
      .finally(() => setIsLoading(false));
  }, [open, meetingId]);

  const selectedVersion = versions.find((v) => v.version === selected);
  const compareVersion = versions.find((v) => v.version === compareWith);
  const diff = useMemo(
    () => (selectedVersion && compareVersion ? diffLines(compareVersion.markdown, selectedVersion.markdown) : null),
    [selectedVersion, compareVersion],
  );

  const handleRestore = async () => {
    if (selected === null) return;
    setIsRestoring(true);
    try {
      await invoke('api_restore_summary_version', { meetingId, version: selected });
      toast.success(`已恢复第 ${selected} 版`);
      await onRestored();
      onOpenChange(false);
    } catch (error) {
      toast.error('恢复失败', { description: String(error) });
    } finally {
      setIsRestoring(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !isRestoring && onOpenChange(next)}>
      <DialogContent className="sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle>纪要版本</DialogTitle>
          <DialogDescription>
            每次生成的纪要都会保存为一个版本。恢复某一版时，当前手工改过的内容也会先存为一个版本。
          </DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <div className="flex items-center gap-2 text-sm text-gray-500 py-8 justify-center">
            <Loader2 className="w-4 h-4 animate-spin" /> 正在读取…
          </div>
        ) : versions.length === 0 ? (
          <p className="text-sm text-gray-500 py-8 text-center">还没有保存的版本。</p>
        ) : (
          <div className="flex gap-4 h-[60vh]">
            <div className="w-48 flex-shrink-0 overflow-y-auto space-y-1">
              {versions.map((v) => (
                <button
                  key={v.version}
                  onClick={() => {
                    setSelected(v.version);
                    if (compareWith === v.version) setCompareWith(null);
                  }}
                  className={`w-full text-left rounded-md px-3 py-2 text-sm border ${
                    v.version === selected ? 'border-gray-900 bg-gray-50' : 'border-transparent hover:bg-gray-50'
                  }`}
                >
                  <div className="font-medium text-gray-900">
                    第 {v.version} 版{v.source === 'edited' ? '（手工修改）' : ''}
                  </div>
                  <div className="text-xs text-gray-500">{formatTime(v.created_at)}</div>
                </button>
              ))}
            </div>

            <div className="flex-1 min-w-0 flex flex-col">
              <div className="flex items-center gap-2 mb-2 text-sm text-gray-600">
                <span>对比：</span>
                <select
                  className="border rounded px-2 py-1 text-sm"
                  value={compareWith ?? ''}
                  onChange={(e) => setCompareWith(e.target.value ? Number(e.target.value) : null)}
                >
                  <option value="">不对比</option>
                  {versions
                    .filter((v) => v.version !== selected)
                    .map((v) => (
                      <option key={v.version} value={v.version}>
                        与第 {v.version} 版对比
                      </option>
                    ))}
                </select>
                {diff && (
                  <span className="text-xs">
                    <span className="text-green-700">绿色</span>为第 {selected} 版新增，
                    <span className="text-red-700">红色</span>为第 {compareWith} 版有而本版没有
                  </span>
                )}
              </div>
              <div className="flex-1 overflow-y-auto rounded-md border bg-gray-50 p-3 text-sm font-mono whitespace-pre-wrap break-words">
                {diff
                  ? diff.map((line, index) => (
                      <div
                        key={index}
                        className={
                          line.kind === 'added'
                            ? 'bg-green-100 text-green-900'
                            : line.kind === 'removed'
                              ? 'bg-red-100 text-red-900 line-through'
                              : ''
                        }
                      >
                        {line.text || ' '}
                      </div>
                    ))
                  : selectedVersion?.markdown}
              </div>
            </div>
          </div>
        )}

        <DialogFooter>
          {readOnly && <span className="text-xs text-gray-500 mr-auto self-center">纪要已定稿，取消定稿后才能恢复其他版本。</span>}
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isRestoring}>
            关闭
          </Button>
          <Button onClick={handleRestore} disabled={readOnly || isRestoring || selected === null || versions.length === 0}>
            {isRestoring && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
            恢复此版本
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
