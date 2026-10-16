CREATE TABLE document_version (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id TEXT NOT NULL,
    text TEXT NOT NULL,
    language TEXT,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX document_version_doc ON document_version (doc_id, created_at);
