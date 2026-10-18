import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export type ContextSaveState = 'idle' | 'saving' | 'saved' | 'error';

const SAVE_DELAY_MS = 800;

/**
 * The background information typed for a meeting's minutes. Saved automatically while typing,
 * so it needs no submit button and is still there after leaving the meeting.
 */
export function useMeetingContext(meetingId: string) {
  const [text, setTextState] = useState('');
  const [saveState, setSaveState] = useState<ContextSaveState>('idle');
  const pending = useRef<{ meetingId: string; text: string } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const flush = useCallback(async () => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    const job = pending.current;
    if (!job) return;
    pending.current = null;
    try {
      await invoke('meeting_context_save', { meetingId: job.meetingId, text: job.text });
      if (job.meetingId === meetingId) setSaveState('saved');
    } catch (error) {
      console.error('Failed to save meeting background:', error);
      if (job.meetingId === meetingId) setSaveState('error');
    }
  }, [meetingId]);

  useEffect(() => {
    let cancelled = false;
    setTextState('');
    setSaveState('idle');
    invoke<string>('meeting_context_get', { meetingId })
      .then((saved) => {
        if (!cancelled && !pending.current) setTextState(saved);
      })
      .catch((error) => console.error('Failed to load meeting background:', error));
    return () => {
      cancelled = true;
      void flush();
    };
  }, [meetingId, flush]);

  const setText = useCallback((value: string) => {
    setTextState(value);
    setSaveState('saving');
    pending.current = { meetingId, text: value };
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void flush(), SAVE_DELAY_MS);
  }, [meetingId, flush]);

  return { text, setText, saveState };
}
