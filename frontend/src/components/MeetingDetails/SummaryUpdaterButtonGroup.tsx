"use client";

import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, Save, Loader2, FileDown } from 'lucide-react';
import Analytics from '@/lib/analytics';

interface SummaryUpdaterButtonGroupProps {
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
  /** Hide saving once the minutes are final */
  readOnly?: boolean;
  onExportDocx?: () => Promise<void>;
  onExportPdf?: () => Promise<void>;
  isExporting?: boolean;
}

export function SummaryUpdaterButtonGroup({
  isSaving,
  isDirty,
  onSave,
  onCopy,
  readOnly = false,
  onExportDocx,
  onExportPdf,
  isExporting = false,
}: SummaryUpdaterButtonGroupProps) {
  return (
    <ButtonGroup>
      {/* Save button */}
      {!readOnly && <Button
        variant="outline"
        size="sm"
        className={`${isDirty ? 'bg-green-200' : ""}`}
        title={isSaving ? "正在保存" : "保存更改"}
        onClick={() => {
          Analytics.trackButtonClick('save_changes', 'meeting_details');
          onSave();
        }}
        disabled={isSaving}
      >
        {isSaving ? (
          <>
            <Loader2 className="animate-spin" />
            <span className="hidden @[40rem]:inline">正在保存…</span>
          </>
        ) : (
          <>
            <Save />
            <span className="hidden @[40rem]:inline">保存</span>
          </>
        )}
      </Button>}

      {/* Copy button */}
      <Button
        variant="outline"
        size="sm"
        title="复制纪要"
        onClick={() => {
          Analytics.trackButtonClick('copy_summary', 'meeting_details');
          onCopy();
        }}
        className="cursor-pointer"
      >
        <Copy />
        <span className="hidden @[40rem]:inline">复制</span>
      </Button>

      {onExportDocx && (
        <Button
          variant="outline"
          size="sm"
          title="按公文格式导出 Word 文档"
          onClick={() => onExportDocx()}
          disabled={isExporting}
        >
          {isExporting ? <Loader2 className="animate-spin" /> : <FileDown />}
          <span className="hidden @[40rem]:inline">导出 Word</span>
        </Button>
      )}

      {onExportPdf && (
        <Button
          variant="outline"
          size="sm"
          title="导出 PDF：用本机的 Word、WPS 或 LibreOffice 把公文格式的 Word 文档转成 PDF"
          onClick={() => onExportPdf()}
          disabled={isExporting}
        >
          {isExporting ? <Loader2 className="animate-spin" /> : <FileDown />}
          <span className="hidden @[40rem]:inline">导出 PDF</span>
        </Button>
      )}

    </ButtonGroup>
  );
}
