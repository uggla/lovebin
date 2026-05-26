DROP TABLE IF EXISTS hearts;
DROP TABLE IF EXISTS heart_votes;

CREATE TABLE heart_votes (
    voter_id TEXT PRIMARY KEY,
    last_voted_at TEXT NOT NULL
);

CREATE TABLE heart_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    voted_at TEXT NOT NULL
);
