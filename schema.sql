CREATE TABLE IF NOT EXISTS bins (
    id TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bin_id TEXT NOT NULL,
    method TEXT NOT NULL,
    headers TEXT,
    body TEXT,
    timestamp TEXT NOT NULL,
    FOREIGN KEY (bin_id) REFERENCES bins(id)
);
