"use client";

import { Transcript, TranscriptSegmentData } from '@/types';
import { TranscriptView } from '@/components/TranscriptView';
import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { TranscriptButtonGroup } from './TranscriptButtonGroup';
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import type { ContextSaveState } from '@/hooks/meeting-details/useMeetingContext';
import { reviewReasons, type EditedLine } from '@/lib/transcript-review';
import type { TranscriptReviewActions } from './ReviewableLine';

const SAVE_STATE_TEXT: Record<ContextSaveState, string> = {
  idle: '自动保存，下次生成纪要时使用，不需要提交',
  saving: '正在保存…',
  saved: '已保存，下次生成纪要时使用',
  error: '保存失败，本次生成仍会使用，但离开页面后不会保留',
};

interface TranscriptPanelProps {
  transcripts: Transcript[];
  customPrompt: string;
  onPromptChange: (value: string) => void;
  promptSaveState?: ContextSaveState;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  isRecording: boolean;
  disableAutoScroll?: boolean;

  // Optional pagination props (when using virtualization)
  usePagination?: boolean;
  segments?: TranscriptSegmentData[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;

  // Retranscription props
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;

  // Line-by-line review: given together, lines can be checked and edited
  isFinalized?: boolean;
  onLineUpdated?: (line: EditedLine) => void;
  onLoadAll?: () => Promise<void>;
  /** Line to scroll to and highlight, e.g. the source of a decision */
  focusLine?: { id: string; nonce: number } | null;
}

export function TranscriptPanel({
  transcripts,
  customPrompt,
  onPromptChange,
  promptSaveState = 'idle',
  onCopyTranscript,
  onOpenMeetingFolder,
  isRecording,
  disableAutoScroll = false,
  usePagination = false,
  segments,
  hasMore,
  isLoadingMore,
  totalCount,
  loadedCount,
  onLoadMore,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
  isFinalized = false,
  onLineUpdated,
  onLoadAll,
  focusLine,
}: TranscriptPanelProps) {
  const [reviewOnly, setReviewOnly] = useState(false);
  // Convert transcripts to segments if pagination is not used but we want virtualization
  const convertedSegments = useMemo(() => {
    if (usePagination && segments) {
      return segments;
    }
    // Convert transcripts to segments for virtualization
    return transcripts.map(t => ({
      id: t.id,
      timestamp: t.audio_start_time ?? 0,
      endTime: t.audio_end_time,
      text: t.text,
      confidence: t.confidence,
    }));
  }, [transcripts, usePagination, segments]);

  const reviewEnabled = Boolean(meetingId && onLineUpdated) && !isRecording;
  const toReview = useMemo(
    () => convertedSegments.filter((segment) => reviewReasons(segment).length > 0),
    [convertedSegments],
  );

  // The filter has to see every line, not only the pages loaded so far
  useEffect(() => {
    if (reviewOnly && hasMore && onLoadAll) void onLoadAll();
  }, [reviewOnly, hasMore, onLoadAll]);

  useEffect(() => setReviewOnly(false), [meetingId]);

  // The focused line may be filtered out or not loaded yet
  useEffect(() => {
    if (!focusLine) return;
    if (convertedSegments.some((segment) => segment.id === focusLine.id)) {
      if (reviewOnly && !toReview.some((segment) => segment.id === focusLine.id)) setReviewOnly(false);
    } else if (hasMore && onLoadAll) {
      void onLoadAll();
    }
    // Only when a new line is asked for
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusLine]);

  const review = useMemo<TranscriptReviewActions | undefined>(() => {
    if (!reviewEnabled || !meetingId || !onLineUpdated) return undefined;
    const call = async (command: string, args: Record<string, unknown>, failure: string) => {
      try {
        onLineUpdated(await invoke<EditedLine>(command, { meetingId, ...args }));
        return true;
      } catch (error) {
        toast.error(failure, { description: String(error) });
        return false;
      }
    };
    return {
      lockedReason: isFinalized ? '纪要已定稿，记录稿不能修改；要修改请先取消定稿' : undefined,
      onSave: (lineId, text) => call('transcript_update_line', { lineId, text }, '保存失败'),
      onConfirm: async (lineId) => { await call('transcript_confirm_line', { lineId }, '确认失败'); },
      onRestore: async (lineId) => { await call('transcript_restore_line', { lineId }, '恢复失败'); },
    };
  }, [reviewEnabled, meetingId, onLineUpdated, isFinalized]);

  const shownSegments = reviewOnly ? toReview : convertedSegments;

  return (
    <div className="flex h-full min-w-0 w-full bg-white flex-col relative @container">
      {/* Title area */}
      <div className="p-4 border-b border-gray-200">
        <TranscriptButtonGroup
          transcriptCount={usePagination ? (totalCount ?? convertedSegments.length) : (transcripts?.length || 0)}
          onCopyTranscript={onCopyTranscript}
          onOpenMeetingFolder={onOpenMeetingFolder}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onRefetchTranscripts={onRefetchTranscripts}
        />
      </div>

      {review && convertedSegments.length > 0 && (
        <div className="flex items-center gap-3 border-b border-gray-100 px-4 py-1.5 text-xs text-gray-500">
          <label className="flex cursor-pointer items-center gap-1.5">
            <input type="checkbox" checked={reviewOnly} onChange={(e) => setReviewOnly(e.target.checked)} />
            只看待复核（{toReview.length}{hasMore ? '+' : ''}）
          </label>
          {review.lockedReason ? (
            <span>{review.lockedReason}</span>
          ) : (
            <span className="text-gray-400">把握度低或过短的句子会标出来；改过的句子可以恢复原文</span>
          )}
        </div>
      )}

      {/* Transcript content - use virtualized view for better performance */}
      <div className="flex-1 overflow-hidden pb-4">
        {reviewOnly && shownSegments.length === 0 ? (
          <p className="mt-8 text-center text-sm text-gray-500">
            {isLoadingMore ? '正在加载全部记录稿…' : '没有待复核的句子了'}
          </p>
        ) : (
        <VirtualizedTranscriptView
          segments={shownSegments}
          isRecording={isRecording}
          isPaused={false}
          isProcessing={false}
          isStopping={false}
          enableStreaming={false}
          showConfidence={true}
          disableAutoScroll={disableAutoScroll}
          hasMore={hasMore}
          isLoadingMore={isLoadingMore}
          totalCount={totalCount}
          loadedCount={loadedCount}
          onLoadMore={onLoadMore}
          review={review}
          focus={focusLine}
        />
        )}
      </div>

      {/* Custom prompt input at bottom of transcript section */}
      {!isRecording && convertedSegments.length > 0 && (
        <div className="p-1 border-t border-gray-200">
          <textarea
            placeholder="为 AI 纪要补充背景信息，例如参会人员、会议概况、会议目标等。输入后自动保存，下次点「生成纪要」时使用。"
            className="w-full px-3 py-2 border border-gray-200 rounded-md text-sm focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500 bg-white shadow-sm min-h-[80px] resize-y"
            value={customPrompt}
            onChange={(e) => onPromptChange(e.target.value)}
          />
          {customPrompt.trim() && (
            <p className={`px-1 text-xs ${promptSaveState === 'error' ? 'text-red-600' : 'text-gray-400'}`}>
              {SAVE_STATE_TEXT[promptSaveState]}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
