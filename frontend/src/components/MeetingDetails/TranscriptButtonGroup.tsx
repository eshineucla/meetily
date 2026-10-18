"use client";

import { useState, useCallback } from 'react';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, FolderOpen, RefreshCw, BookA } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import type { CorrectionHit } from '@/components/GlossarySettings';
import Analytics from '@/lib/analytics';
import { RetranscribeDialog } from './RetranscribeDialog';
import { useConfig } from '@/contexts/ConfigContext';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  const { betaFeatures } = useConfig();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);
  const [applyingGlossary, setApplyingGlossary] = useState(false);

  // New glossary entries only apply to transcripts made afterwards; this applies them here too
  const handleApplyGlossary = useCallback(async () => {
    if (!meetingId) return;
    setApplyingGlossary(true);
    try {
      const result = await invoke<{ hits: CorrectionHit[]; segmentsChanged: number }>('glossary_apply_to_meeting', { meetingId });
      if (result.segmentsChanged === 0) {
        toast.info('记录稿里没有需要按术语库纠正的地方');
      } else {
        toast.success(`已纠正 ${result.hits.reduce((n, h) => n + h.count, 0)} 处`, {
          description: result.hits.map((h) => `「${h.alias}」→「${h.canonical}」${h.count} 处`).join('；'),
          duration: 8000,
        });
        await onRefetchTranscripts?.();
      }
    } catch (error) {
      toast.error('按术语库纠正失败', { description: String(error) });
    } finally {
      setApplyingGlossary(false);
    }
  }, [meetingId, onRefetchTranscripts]);

  const handleRetranscribeComplete = useCallback(async () => {
    // Refetch transcripts to show the updated data
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  return (
    <div className="flex items-center justify-center w-full gap-2">
      <ButtonGroup>
        <Button
          variant="outline"
          size="sm"
          className="px-2 @[22rem]:px-3"
          onClick={() => {
            Analytics.trackButtonClick('copy_transcript', 'meeting_details');
            onCopyTranscript();
          }}
          disabled={transcriptCount === 0}
          title={transcriptCount === 0 ? '暂无转写稿' : '复制转写稿'}
        >
          <Copy />
          <span className="hidden @[22rem]:inline">复制</span>
        </Button>

        <Button
          size="sm"
          variant="outline"
          className="px-2 @[22rem]:px-4"
          onClick={() => {
            Analytics.trackButtonClick('open_recording_folder', 'meeting_details');
            onOpenMeetingFolder();
          }}
          title="打开录音文件夹"
        >
          <FolderOpen className="@[22rem]:mr-2" size={18} />
          <span className="hidden @[22rem]:inline">录音</span>
        </Button>

        {meetingId && (
          <Button
            size="sm"
            variant="outline"
            className="px-2 @[22rem]:px-4"
            onClick={handleApplyGlossary}
            disabled={transcriptCount === 0 || applyingGlossary}
            title="按术语库纠正这场会议的记录稿（术语库在 设置 → 术语库 维护）"
          >
            <BookA className="@[22rem]:mr-2" size={18} />
            <span className="hidden @[22rem]:inline">{applyingGlossary ? '纠正中' : '术语'}</span>
          </Button>
        )}

        {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="outline"
            className="bg-gradient-to-r from-blue-50 to-purple-50 hover:from-blue-100 hover:to-purple-100 border-blue-200 px-2 @[22rem]:px-4"
            onClick={() => {
              Analytics.trackButtonClick('enhance_transcript', 'meeting_details');
              setShowRetranscribeDialog(true);
            }}
            title="重新转写录音，提升转写质量"
          >
            <RefreshCw className="@[22rem]:mr-2" size={18} />
            <span className="hidden @[22rem]:inline">增强</span>
          </Button>
        )}
      </ButtonGroup>

      {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
