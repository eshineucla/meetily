/**
 * Pre-finalization check of meeting minutes (M2-02).
 *
 * Plain rules, no model involved, so every flag can be explained:
 * - each task in a task section/table needs an owner and a deadline
 * - each decision and task needs a source: a verbatim quote from the transcript,
 *   written as （原文：“……”） or in a 原文依据 table column
 * - each quote has to be findable in the transcript
 * - the basic facts (time, place, chair, attendees, topics) should be filled in, when the minutes
 *   list them as "项目：内容" lines
 * - no colloquial or vague wording anywhere (the list from the old workbench's templates)
 */

export type MinutesIssue = 'no-owner' | 'no-deadline' | 'no-source' | 'source-not-found' | 'not-filled' | 'colloquial';

export const ISSUE_LABELS: Record<MinutesIssue, string> = {
  'no-owner': '没有责任人',
  'no-deadline': '没有完成时限',
  'no-source': '没有原文出处',
  'source-not-found': '原文片段在记录稿中找不到',
  'not-filled': '要素未填写',
  'colloquial': '口语或含糊表述',
};

export interface CheckedItem {
  section: string;
  text: string;
  quote?: string;
  issues: MinutesIssue[];
}

export interface MinutesCheckResult {
  items: CheckedItem[];
  issueCount: number;
}

const DECISION_SECTION = /议定事项|决定|决议|结论|Decision/i;
const TASK_SECTION = /分工|落实|待办|任务|行动|要求|工作安排|Action|Next Steps/i;
const BASICS_SECTION = /基本情况|会议概况/;
const BASIC_FIELD = /^(会议时间|会议地点|主持人|参会人员|出席人员|会议议题)\s*[:：]\s*(.*)$/;

/** Wording that does not belong in minutes */
export const COLLOQUIAL_PHRASES = ['大家觉得', '差不多', '可能吧', '应该是', '反正', '搞定', '弄一下', '有点问题'];

const OWNER_HEADER = /责任|负责|承办|Owner/i;
const DEADLINE_HEADER = /时限|期限|截止|完成时间|日期|Due/i;
const SOURCE_HEADER = /原文|出处|依据|Reference/i;
const TASK_HEADER = /事项|任务|工作|内容|行动|Task|Action/i;

const EMPTY_VALUES = new Set(['', '-', '—', '——', '/', '无', '未明确', '待定', '未提及', '不详', 'n/a', 'na', 'tbd']);

const INLINE_QUOTE = /(?:原文|出处|依据)\s*[:：]\s*[“"「『]([^”"」』]+)[”"」』]/;

const isEmptyValue = (value: string) => EMPTY_VALUES.has(stripMarkup(value).toLowerCase());

function stripMarkup(text: string): string {
  return text.replace(/\*\*|__|`/g, '').replace(/<br\s*\/?>/gi, ' ').trim();
}

/** Drop whitespace and punctuation so small formatting differences don't matter */
export function normalizeForMatch(text: string): string {
  return text.toLowerCase().replace(/[\s\p{P}\p{S}]/gu, '');
}

function bigrams(text: string): string[] {
  const chars = Array.from(text);
  const result: string[] = [];
  for (let i = 0; i + 1 < chars.length; i++) result.push(chars[i] + chars[i + 1]);
  return result;
}

/**
 * Whether the quote appears in the transcript: exactly once punctuation and spacing are ignored,
 * or with at least 80% of its two-character pieces present (to tolerate small transcription slips).
 */
export function quoteFoundIn(quote: string, normalizedTranscript: string, transcriptBigrams: Set<string>): boolean {
  const normalized = normalizeForMatch(quote);
  if (!normalized) return false;
  if (normalizedTranscript.includes(normalized)) return true;
  const pieces = bigrams(normalized);
  if (pieces.length < 4) return false;
  const present = pieces.filter((piece) => transcriptBigrams.has(piece)).length;
  return present / pieces.length >= 0.8;
}

function splitRow(line: string): string[] {
  return line.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map((cell) => cell.trim());
}

const isSeparatorRow = (line: string) => /^\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?$/.test(line.trim());

const LIST_ITEM = /^\s*(?:[-*+]|\d+[.、)）]|[（(]\d+[)）]|[一二三四五六七八九十]+、)\s+(.*)$/;

export function checkMinutes(markdown: string, transcript: string): MinutesCheckResult {
  const normalizedTranscript = normalizeForMatch(transcript);
  const transcriptBigrams = new Set(bigrams(normalizedTranscript));
  const items: CheckedItem[] = [];

  const sourceIssues = (quote: string | undefined): MinutesIssue[] => {
    if (!quote || isEmptyValue(quote)) return ['no-source'];
    return quoteFoundIn(quote, normalizedTranscript, transcriptBigrams) ? [] : ['source-not-found'];
  };

  const lines = markdown.split('\n');
  let section = '';
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    if (heading) {
      section = stripMarkup(heading[1]);
      i++;
      continue;
    }

    const plain = stripMarkup(line.replace(LIST_ITEM, '$1'));
    // Quotes from the transcript are spoken words, so they don't count
    const ownWords = plain.replace(/[（(]\s*(?:原文|出处|依据)\s*[:：][^）)]*[）)]/g, '');
    const colloquial = COLLOQUIAL_PHRASES.filter((phrase) => ownWords.includes(phrase));
    if (colloquial.length > 0) {
      items.push({ section, text: `${plain}（${colloquial.join('、')}）`, issues: ['colloquial'] });
    }

    if (BASICS_SECTION.test(section)) {
      const field = plain.match(BASIC_FIELD);
      if (field) {
        const value = field[2].replace(/[。；;]$/, '');
        items.push({ section, text: `${field[1]}：${value || '（空）'}`, issues: isEmptyValue(value) ? ['not-filled'] : [] });
      }
      i++;
      continue;
    }

    const isDecision = DECISION_SECTION.test(section);
    const isTask = TASK_SECTION.test(section);
    if (!isDecision && !isTask) {
      i++;
      continue;
    }

    // Table: header, separator, rows
    if (line.trim().startsWith('|') && i + 1 < lines.length && isSeparatorRow(lines[i + 1])) {
      const header = splitRow(line);
      const findColumn = (pattern: RegExp) => header.findIndex((cell) => pattern.test(cell));
      const ownerCol = findColumn(OWNER_HEADER);
      const deadlineCol = findColumn(DEADLINE_HEADER);
      const sourceCol = findColumn(SOURCE_HEADER);
      const taskCol = findColumn(TASK_HEADER);
      i += 2;
      while (i < lines.length && lines[i].trim().startsWith('|')) {
        const cells = splitRow(lines[i]);
        i++;
        if (cells.every((cell) => isEmptyValue(cell))) continue;
        const text = stripMarkup(taskCol >= 0 ? cells[taskCol] ?? '' : cells.join(' / '));
        const sourceCell = sourceCol >= 0 ? stripMarkup(cells[sourceCol] ?? '') : '';
        const quote = sourceCell.replace(/^[“"「『]|[”"」』]$/g, '') || text.match(INLINE_QUOTE)?.[1];
        const issues: MinutesIssue[] = [];
        if (isTask && (ownerCol < 0 || isEmptyValue(cells[ownerCol] ?? ''))) issues.push('no-owner');
        if (isTask && (deadlineCol < 0 || isEmptyValue(cells[deadlineCol] ?? ''))) issues.push('no-deadline');
        issues.push(...sourceIssues(quote));
        items.push({ section, text, quote, issues });
      }
      continue;
    }

    const listItem = line.match(LIST_ITEM);
    if (listItem) {
      const text = stripMarkup(listItem[1]);
      if (text && !isEmptyValue(text)) {
        const quote = text.match(INLINE_QUOTE)?.[1];
        const issues: MinutesIssue[] = [];
        if (isTask) {
          if (!/责任|负责|牵头|承办/.test(text)) issues.push('no-owner');
          if (!/前|内|底|截止|时限|\d+\s*月|\d+\s*日|本周|下周|月底|年底/.test(text)) issues.push('no-deadline');
        }
        issues.push(...sourceIssues(quote));
        items.push({ section, text, quote, issues });
      }
    }
    i++;
  }

  return { items, issueCount: items.filter((item) => item.issues.length > 0).length };
}
