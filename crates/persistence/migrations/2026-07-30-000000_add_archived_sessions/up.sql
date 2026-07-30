CREATE TABLE archived_sessions (
    id TEXT NOT NULL PRIMARY KEY,
    title TEXT NOT NULL,
    cwd TEXT,
    cli_agent TEXT,
    branch TEXT,
    last_used_at TIMESTAMP NOT NULL,
    archived_at TIMESTAMP NOT NULL
);
