-- Transcript review: the recognition confidence of each line (to flag lines worth checking),
-- the text as transcribed before the user first edited it (kept so it can be restored), and when
-- the user checked the line, by editing it or by confirming it as transcribed.
ALTER TABLE transcripts ADD COLUMN confidence REAL;
ALTER TABLE transcripts ADD COLUMN original_text TEXT;
ALTER TABLE transcripts ADD COLUMN reviewed_at TEXT;
