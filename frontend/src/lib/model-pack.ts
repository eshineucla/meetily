import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { toast } from 'sonner';

export interface ModelPackImportReport {
  imported: string[];
  already_present: string[];
  ignored: string[];
  rejected: string[];
}

export interface ModelPackCandidate {
  id: string;
  label: string;
  kind: 'whisper' | 'onnx' | 'summary';
  bytes: number;
}

export interface ModelPackProgress {
  phase: 'import' | 'export';
  doneBytes: number;
  totalBytes: number;
  current: string;
}

/** Picks a model pack and installs it. Returns null when the user cancels; shows the outcome as a toast. */
export async function importModelPack(): Promise<ModelPackImportReport | null> {
  try {
    const report = await invoke<ModelPackImportReport | null>('model_pack_import');
    if (!report) return null;
    const notes = [
      report.already_present.length > 0 ? `已存在：${report.already_present.join('、')}` : '',
      report.rejected.length > 0 ? `未导入：${report.rejected.join('；')}` : '',
    ].filter(Boolean).join('\n');
    if (report.imported.length > 0) {
      toast.success(`已导入 ${report.imported.join('、')}`, { description: notes || undefined, duration: 8000 });
    } else if (report.already_present.length > 0 && report.rejected.length === 0) {
      toast.info('模型包里的模型都已安装', { description: notes });
    } else {
      toast.warning('模型包里没有可用的模型', {
        description: notes || '请确认这是会议纪要导出的模型包。',
        duration: 10000,
      });
    }
    return report;
  } catch (error) {
    toast.error('导入模型包失败', { description: String(error) });
    throw error;
  }
}

/** Latest model pack progress while an import or export runs */
export function useModelPackProgress(): ModelPackProgress | null {
  const [progress, setProgress] = useState<ModelPackProgress | null>(null);
  useEffect(() => {
    const unlisten = listen<ModelPackProgress>('model-pack-progress', (event) => setProgress(event.payload));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);
  return progress;
}

export function formatPackSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}
