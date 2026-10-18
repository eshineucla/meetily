import type { TranscriptSegmentData } from '@/types';

/** Lines recognised below this confidence are worth checking against the recording */
export const LOW_CONFIDENCE = 0.7;

/** Why a line is worth checking; empty when it is not, or when the user already checked it */
export function reviewReasons(segment: TranscriptSegmentData): string[] {
  if (segment.reviewed) return [];
  const reasons: string[] = [];
  if (segment.confidence !== undefined && segment.confidence < LOW_CONFIDENCE) {
    reasons.push(`识别把握度 ${Math.round(segment.confidence * 100)}%`);
  }
  if (Array.from(segment.text.trim()).length <= 1) {
    reasons.push('内容过短，可能是杂音');
  }
  return reasons;
}

/** Saved state of a line after an edit, confirmation or restore */
export interface EditedLine {
  id: string;
  text: string;
  originalText?: string | null;
  reviewed: boolean;
}
