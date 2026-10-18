'use client';

import { memo, useEffect, useRef, useState } from 'react';
import { Check, Pencil, Undo2 } from 'lucide-react';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { reviewReasons } from '@/lib/transcript-review';
import type { TranscriptSegmentData } from '@/types';

export interface TranscriptReviewActions {
  /** Why lines can't be changed right now, e.g. the minutes are finalized */
  lockedReason?: string;
  /** Resolves true when saved, so the editor can close */
  onSave: (id: string, text: string) => Promise<boolean>;
  onConfirm: (id: string) => Promise<void>;
  onRestore: (id: string) => Promise<void>;
}

const ACTION_CLASS = 'inline-flex items-center gap-0.5 rounded px-1.5 py-0.5 text-gray-500 hover:bg-gray-100 hover:text-gray-800';

function formatTime(seconds: number): string {
  const total = Math.floor(seconds);
  return `[${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}]`;
}

/** One transcript line on the meeting page: shows why it may need checking, and can be edited */
export const ReviewableLine = memo(function ReviewableLine({
  segment,
  actions,
}: {
  segment: TranscriptSegmentData;
  actions: TranscriptReviewActions;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(segment.text);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const reasons = reviewReasons(segment);
  const edited = segment.originalText !== undefined && segment.originalText !== null;
  const locked = Boolean(actions.lockedReason);
  const hasTags = reasons.length > 0 || edited || Boolean(segment.reviewed);

  useEffect(() => {
    if (editing) {
      const input = inputRef.current;
      input?.focus();
      input?.setSelectionRange(input.value.length, input.value.length);
    }
  }, [editing]);

  const startEditing = () => {
    setDraft(segment.text);
    setEditing(true);
  };

  const run = async (work: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await work();
    } finally {
      setBusy(false);
    }
  };

  const editButton = (
    <button onClick={startEditing} disabled={busy} className={ACTION_CLASS}>
      <Pencil className="h-3 w-3" />修改
    </button>
  );

  const save = () => run(async () => {
    if (await actions.onSave(segment.id, draft)) setEditing(false);
  });

  return (
    <div id={`segment-${segment.id}`} className="group relative mb-3">
      <div className="flex items-start gap-2">
        <span className="text-xs text-gray-400 mt-1 flex-shrink-0 min-w-[50px]">{formatTime(segment.timestamp)}</span>
        <div className="flex-1 min-w-0">
          {editing ? (
            <div>
              <textarea
                ref={inputRef}
                value={draft}
                disabled={busy}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    void save();
                  } else if (e.key === 'Escape') {
                    setEditing(false);
                  }
                }}
                rows={Math.min(6, Math.max(2, Math.ceil(draft.length / 40)))}
                className="w-full rounded-md border border-blue-300 px-2 py-1 text-base leading-relaxed text-gray-800 focus:outline-none focus:ring-1 focus:ring-blue-500"
              />
              <div className="mt-1 flex items-center gap-2 text-xs">
                <button
                  onClick={() => void save()}
                  disabled={busy || !draft.trim()}
                  className="rounded bg-blue-600 px-2 py-0.5 text-white hover:bg-blue-700 disabled:opacity-50"
                >
                  保存
                </button>
                <button onClick={() => setEditing(false)} disabled={busy} className="rounded px-2 py-0.5 text-gray-600 hover:bg-gray-100">
                  取消
                </button>
                <span className="text-gray-400">回车保存，Shift+回车换行，Esc 取消</span>
              </div>
              {edited && <p className="mt-1 text-xs text-gray-400">转写原文：{segment.originalText}</p>}
            </div>
          ) : (
            <>
              <p className="text-base text-gray-800 leading-relaxed whitespace-pre-wrap break-words">
                {segment.text.trim() === '' ? '[静音]' : segment.text}
              </p>
              {hasTags ? (
                <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs">
                  {reasons.map((reason) => (
                    <span key={reason} className="rounded bg-amber-50 px-1.5 py-0.5 text-amber-700">
                      待复核：{reason}
                    </span>
                  ))}
                  {edited && (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="cursor-default rounded bg-blue-50 px-1.5 py-0.5 text-blue-700">已修改</span>
                      </TooltipTrigger>
                      <TooltipContent className="max-w-sm">转写原文：{segment.originalText}</TooltipContent>
                    </Tooltip>
                  )}
                  {!edited && segment.reviewed && (
                    <span className="rounded bg-green-50 px-1.5 py-0.5 text-green-700">已确认</span>
                  )}
                  {!locked && (
                    <>
                      {editButton}
                      {reasons.length > 0 && (
                        <button onClick={() => void run(() => actions.onConfirm(segment.id))} disabled={busy} className={ACTION_CLASS}>
                          <Check className="h-3 w-3" />确认无误
                        </button>
                      )}
                      {(edited || segment.reviewed) && (
                        <button onClick={() => void run(() => actions.onRestore(segment.id))} disabled={busy} className={ACTION_CLASS}>
                          <Undo2 className="h-3 w-3" />{edited ? '恢复原文' : '取消确认'}
                        </button>
                      )}
                    </>
                  )}
                </div>
              ) : !locked && (
                <div className="absolute right-0 top-0 hidden bg-white text-xs group-hover:block group-focus-within:block">
                  {editButton}
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
});
