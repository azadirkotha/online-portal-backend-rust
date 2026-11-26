-- Add migration script here
-- Visitor comments. One level of replies only:
--   parent_id IS NULL      -> top-level comment
--   parent_id = <comment>  -> reply (always points at a TOP-LEVEL comment, never at another reply)
-- reply_to keeps the name of the person being answered when someone replies to a reply,
-- so the UI can show "↪ Name" without creating another nesting level.
CREATE TABLE IF NOT EXISTS comments (
    id INT AUTO_INCREMENT PRIMARY KEY,
    article_id INT NOT NULL,
    parent_id INT NULL,
    reply_to VARCHAR(100) NULL,
    name VARCHAR(100) NOT NULL,
    body TEXT NOT NULL,
    -- approved | pending | rejected  (new comments use NEW_STATUS in models/public_comments.rs)
    status VARCHAR(20) NOT NULL DEFAULT 'approved',
    -- stored in UTC (the API sends it with a trailing Z)
    created_at DATETIME NOT NULL,
    KEY idx_comments_article (article_id, parent_id, status, created_at),
    KEY idx_comments_parent (parent_id, status, created_at),
    CONSTRAINT fk_comments_article FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE,
    -- deleting a top-level comment removes its replies too
    CONSTRAINT fk_comments_parent FOREIGN KEY (parent_id) REFERENCES comments(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;