-- Glossary (术语库): correct spellings of names, organisations and terms, and the ways speech
-- recognition gets them wrong. Aliases are a JSON array. See src/glossary/matcher.rs.
CREATE TABLE IF NOT EXISTS glossary_entry (
    id         TEXT PRIMARY KEY,
    category   TEXT NOT NULL,
    canonical  TEXT NOT NULL,
    aliases    TEXT NOT NULL DEFAULT '[]',
    note       TEXT NOT NULL DEFAULT '',
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (category, canonical)
);
