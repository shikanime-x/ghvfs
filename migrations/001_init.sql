CREATE TABLE IF NOT EXISTS records (
    repo TEXT NOT NULL,
    kind TEXT NOT NULL,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    readonly TEXT NOT NULL,
    PRIMARY KEY (repo, kind, number)
);
